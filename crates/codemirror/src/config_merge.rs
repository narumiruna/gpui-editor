use std::collections::BTreeMap;

/// Combine ordered configuration entries, then fill missing values from defaults.
/// `None` is undefined. Equal values do not call the combiner. For two different
/// defined values, the combiner returns `Some(value)` to set the result, or
/// `None` if the field does not permit a merge. `Some(None)` resets the field.
///
/// The result uses native key order and value equality. It has no inherited
/// properties. On a conflict, return the first conflicting field name.
pub fn combine_config<'a, T: Clone + PartialEq>(
    configs: &[&[(&'a str, Option<T>)]],
    defaults: &[(&'a str, Option<T>)],
    mut combine: impl FnMut(&str, &T, &T) -> Option<Option<T>>,
) -> Result<BTreeMap<&'a str, Option<T>>, &'a str> {
    let mut result: BTreeMap<&str, Option<T>> = BTreeMap::new();
    for (key, value) in configs.iter().flat_map(|entries| entries.iter()) {
        let current = result.entry(key).or_default();
        match (&*current, value) {
            (None, _) => *current = value.clone(),
            (Some(first), Some(next)) if first != next => {
                *current = combine(key, first, next).ok_or(*key)?;
            }
            _ => {}
        }
    }
    for (key, value) in defaults {
        let current = result.entry(key).or_default();
        if current.is_none() {
            *current = value.clone();
        }
    }
    Ok(result)
}
