//! GAS and Z80 streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::LegacyMode;
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod gas;
mod z80;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Gas | LegacyMode::GasArm => "gas",
        LegacyMode::Z80 | LegacyMode::Ez80 => "z80",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let data = LanguageData::default();
    let marker = match mode {
        LegacyMode::Gas => "#",
        LegacyMode::GasArm => "@",
        LegacyMode::Z80 | LegacyMode::Ez80 => return data,
        _ => unreachable!(),
    };
    data.with(
        "commentTokens",
        CommentTokens {
            line: Some(marker),
            block: Some(("/*", "*/")),
        },
    )
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Gas(gas::State),
    GasArm(gas::State),
    Z80(z80::State),
    Ez80(z80::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self(match mode {
            LegacyMode::Gas => Scan::Gas(gas::State::default()),
            LegacyMode::GasArm => Scan::GasArm(gas::State::default()),
            LegacyMode::Z80 => Scan::Z80(z80::State::default()),
            LegacyMode::Ez80 => Scan::Ez80(z80::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.0 {
            Scan::Gas(_) => LegacyMode::Gas,
            Scan::GasArm(_) => LegacyMode::GasArm,
            Scan::Z80(_) => LegacyMode::Z80,
            Scan::Ez80(_) => LegacyMode::Ez80,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Gas(state) => state.token(stream, false),
            Scan::GasArm(state) => state.token(stream, true),
            Scan::Z80(state) => state.token(stream, false),
            Scan::Ez80(state) => state.token(stream, true),
        }
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
