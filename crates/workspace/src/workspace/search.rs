use std::{
    fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{EntryKind, Stamp, Workspace, classify_entry, file_metadata};

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_HITS: usize = 200;
const MAX_MATCHED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// At least one path or file could not be searched in full.
    pub skipped: bool,
    /// A search limit was reached or additional hits were omitted.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// Absolute workspace path.
    pub path: PathBuf,
    /// `None` for a filename match.
    pub location: Option<SearchLocation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchLocation {
    /// One-based line number.
    pub line: usize,
    /// Full line, without its line ending.
    pub text: Arc<str>,
    /// Zero-based UTF-8 byte offset within the line.
    pub column: usize,
}

impl SearchLocation {
    /// Return the absolute UTF-8 byte offset if the source still contains the saved line
    /// at its original line number and the column is valid.
    pub fn offset_in(&self, source: &str) -> Option<usize> {
        let mut offset = 0;
        let mut lines = source.split_inclusive('\n');
        for _ in 0..self.line.checked_sub(1)? {
            offset += lines.next()?.len();
        }
        let text = lines.next()?.lines().next()?;
        (text == self.text.as_ref()
            && self.column < text.len()
            && text.is_char_boundary(self.column))
        .then(|| offset + self.column)
    }
}

impl Workspace {
    /// Search saved `.md` filenames and UTF-8 contents for a case-sensitive literal.
    /// Sort by path, with a filename match before the first match on each matching line.
    /// Empty or multiline queries return no hits. A filename can match even if its contents
    /// cannot be searched.
    /// Read up to 1 MiB per file, scan up to 10,000 entries, and return up to 200 hits.
    /// Retain at most 8 MiB of matching line text across the hits.
    /// Cancellation returns `Interrupted`, not a partial result. Root access errors are fatal.
    pub fn search_markdown(
        &self,
        query: &str,
        canceled: impl Fn() -> bool,
    ) -> io::Result<SearchResults> {
        check_canceled(&canceled)?;
        self.check_root()?;
        let mut results = SearchResults::default();
        if query.is_empty() || query.contains(['\r', '\n']) {
            return Ok(results);
        }
        let mut paths = self.search_paths(&canceled, &mut results)?;
        // All paths have the same prefix, so this also sorts by relative path.
        paths.sort_unstable();
        let mut remaining_bytes = MAX_MATCHED_BYTES;
        'files: for path in paths {
            check_canceled(&canceled)?;
            let stamp = match self.search_stamp(&path) {
                Ok(stamp) => stamp,
                Err(_) => {
                    results.skipped = true;
                    continue;
                }
            };
            let filename_matches = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name.contains(query),
                None => {
                    results.skipped = true;
                    false
                }
            };
            if filename_matches && !results.push(&path, None) {
                break;
            }
            let source = match self.read_search_source(&path, &stamp, &canceled) {
                Ok(source) => source,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(_) => {
                    results.skipped = true;
                    continue;
                }
            };
            for (index, text) in source.lines().enumerate() {
                check_canceled(&canceled)?;
                if let Some(column) = text.find(query) {
                    if text.len() > remaining_bytes {
                        results.truncated = true;
                        break 'files;
                    }
                    if !results.push(&path, Some((index + 1, text, column))) {
                        break 'files;
                    }
                    remaining_bytes -= text.len();
                }
            }
        }
        check_canceled(&canceled)?;
        self.check_root()?;
        Ok(results)
    }

    fn search_paths(
        &self,
        canceled: &impl Fn() -> bool,
        results: &mut SearchResults,
    ) -> io::Result<Vec<PathBuf>> {
        let mut folders = vec![self.root.clone()];
        let mut paths = Vec::new();
        let mut remaining = MAX_ENTRIES;
        while let Some(folder) = folders.pop() {
            check_canceled(canceled)?;
            let at_root = folder == self.root;
            let mut entries = match self
                .check_direct_path(&folder)
                .and_then(|()| fs::read_dir(&folder))
            {
                Ok(entries) => entries,
                Err(error) if at_root => return Err(error),
                Err(_) => {
                    results.skipped = true;
                    continue;
                }
            };
            let (folder_count, path_count) = (folders.len(), paths.len());
            while remaining > 0 {
                check_canceled(canceled)?;
                let Some(entry) = entries.next() else { break };
                remaining -= 1;
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) if at_root => return Err(error),
                    Err(_) => {
                        results.skipped = true;
                        continue;
                    }
                };
                match classify_entry(&entry) {
                    Ok(Some(entry)) if entry.kind == EntryKind::Folder => folders.push(entry.path),
                    Ok(Some(entry)) if entry.kind == EntryKind::Markdown => paths.push(entry.path),
                    Ok(_) => {}
                    Err(_) => results.skipped = true,
                }
            }
            if let Err(error) = self.check_direct_path(&folder) {
                if at_root {
                    return Err(error);
                }
                folders.truncate(folder_count);
                paths.truncate(path_count);
                results.skipped = true;
            }
            if remaining == 0 {
                // Stop without fetching another entry just to determine if there is one.
                results.truncated = true;
                break;
            }
        }
        Ok(paths)
    }

    fn search_stamp(&self, path: &Path) -> io::Result<Stamp> {
        self.check_direct_path(path)?;
        Stamp::read(&file_metadata(path)?)
    }

    fn read_search_source(
        &self,
        path: &Path,
        stamp: &Stamp,
        canceled: &impl Fn() -> bool,
    ) -> io::Result<String> {
        check_canceled(canceled)?;
        if stamp.len > MAX_FILE_BYTES as u64 {
            return Err(io::Error::other("file exceeds search size limit"));
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let mut file = super::publication::open_search_file(self, path)?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let mut file = fs::File::open(path)?;
        if Stamp::read(&file.metadata()?)? != *stamp {
            return Err(super::external_change());
        }
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        while bytes.len() < MAX_FILE_BYTES {
            check_canceled(canceled)?;
            let size = buffer.len().min(MAX_FILE_BYTES - bytes.len());
            match file.read(&mut buffer[..size]) {
                Ok(0) => break,
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        check_canceled(canceled)?;
        if bytes.len() as u64 != stamp.len || self.search_stamp(path)? != *stamp {
            return Err(super::external_change());
        }
        String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

impl SearchResults {
    fn push(&mut self, path: &Path, location: Option<(usize, &str, usize)>) -> bool {
        if self.hits.len() == MAX_HITS {
            self.truncated = true;
            return false;
        }
        self.hits.push(SearchHit {
            path: path.to_owned(),
            location: location.map(|(line, text, column)| SearchLocation {
                line,
                text: Arc::from(text),
                column,
            }),
        });
        true
    }
}

fn check_canceled(canceled: &impl Fn() -> bool) -> io::Result<()> {
    if canceled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "search canceled",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn cancellation_interrupts_a_chunked_read() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(directory.path()).unwrap();
        let path = workspace.root().join("large.md");
        fs::write(&path, vec![b'x'; MAX_FILE_BYTES]).unwrap();
        let stamp = workspace.search_stamp(&path).unwrap();
        let checks = Cell::new(0);
        let error = workspace
            .read_search_source(&path, &stamp, &|| {
                checks.set(checks.get() + 1);
                checks.get() == 4
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
}
