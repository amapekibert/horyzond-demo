//! Configurable modal input state independent of keyboard backends.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// An opaque configured mode name.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModeId(String);
impl ModeId {
    /// Creates a non-empty mode name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name contains no non-whitespace text.
    pub fn new(value: impl Into<String>) -> Result<Self, InputError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(InputError::EmptyMode);
        }
        Ok(Self(value))
    }
    /// Returns the configured mode name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A provider-agnostic command emitted by a configured binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputCommand {
    pub name: String,
    pub arguments: Vec<String>,
}

/// One configured chord binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub chord: String,
    pub command: InputCommand,
    pub next_mode: Option<ModeId>,
}

/// A configured mode and its bindings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mode {
    pub id: ModeId,
    pub bindings: Vec<Binding>,
}

/// Complete data-only modal configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputConfiguration {
    pub initial_mode: ModeId,
    pub modes: Vec<Mode>,
}

/// A command emitted while handling one press.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputOutcome {
    pub consumed: bool,
    pub command: Option<InputCommand>,
    pub mode: ModeId,
}

/// Backend-neutral state that prevents consumed presses leaking after a reload.
#[derive(Debug)]
pub struct InputState {
    modes: BTreeMap<ModeId, Mode>,
    current: ModeId,
    consumed: BTreeSet<String>,
}
impl InputState {
    /// Builds validated input state from configuration.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate modes, invalid bindings, or an unknown
    /// initial mode.
    pub fn new(configuration: InputConfiguration) -> Result<Self, InputError> {
        let modes = index_modes(configuration.modes)?;
        if !modes.contains_key(&configuration.initial_mode) {
            return Err(InputError::UnknownMode(configuration.initial_mode));
        }
        Ok(Self {
            modes,
            current: configuration.initial_mode,
            consumed: BTreeSet::new(),
        })
    }
    /// Returns the selected modal state.
    #[must_use]
    pub fn current_mode(&self) -> &ModeId {
        &self.current
    }
    /// Handles one normalized press and returns only configured commands.
    pub fn press(&mut self, chord: &str) -> InputOutcome {
        let binding = self
            .modes
            .get(&self.current)
            .and_then(|mode| mode.bindings.iter().find(|binding| binding.chord == chord))
            .cloned();
        let Some(binding) = binding else {
            return InputOutcome {
                consumed: false,
                command: None,
                mode: self.current.clone(),
            };
        };
        self.consumed.insert(chord.to_owned());
        if let Some(next) = binding.next_mode {
            self.current = next;
        }
        InputOutcome {
            consumed: true,
            command: Some(binding.command),
            mode: self.current.clone(),
        }
    }
    /// Reports whether a release belongs to a consumed press and clears it.
    pub fn release(&mut self, chord: &str) -> bool {
        self.consumed.remove(chord)
    }
    /// Replaces bindings at a safe boundary and clears consumed input.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate modes, invalid bindings, or when no
    /// valid current or initial mode remains after replacement.
    pub fn replace(&mut self, configuration: InputConfiguration) -> Result<(), InputError> {
        let modes = index_modes(configuration.modes)?;
        let current = if modes.contains_key(&self.current) {
            self.current.clone()
        } else {
            configuration.initial_mode.clone()
        };
        if !modes.contains_key(&current) {
            return Err(InputError::UnknownMode(current));
        }
        self.modes = modes;
        self.current = current;
        self.consumed.clear();
        Ok(())
    }
}

fn index_modes(modes: Vec<Mode>) -> Result<BTreeMap<ModeId, Mode>, InputError> {
    let mut indexed = BTreeMap::new();
    for mode in modes {
        for binding in &mode.bindings {
            if binding.chord.trim().is_empty() || binding.command.name.trim().is_empty() {
                return Err(InputError::InvalidBinding(mode.id.clone()));
            }
        }
        if indexed.insert(mode.id.clone(), mode).is_some() {
            return Err(InputError::DuplicateMode);
        }
    }
    Ok(indexed)
}

/// Input-configuration validation failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputError {
    EmptyMode,
    DuplicateMode,
    UnknownMode(ModeId),
    InvalidBinding(ModeId),
}
impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMode => formatter.write_str("mode name must not be empty"),
            Self::DuplicateMode => formatter.write_str("mode names must be unique"),
            Self::UnknownMode(mode) => write!(formatter, "unknown mode {}", mode.as_str()),
            Self::InvalidBinding(mode) => {
                write!(formatter, "invalid binding in mode {}", mode.as_str())
            }
        }
    }
}
impl std::error::Error for InputError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn mode(name: &str, bindings: Vec<Binding>) -> Mode {
        Mode {
            id: ModeId::new(name).expect("mode"),
            bindings,
        }
    }
    fn binding(chord: &str, action: &str, next: Option<&str>) -> Binding {
        Binding {
            chord: chord.to_owned(),
            command: InputCommand {
                name: action.to_owned(),
                arguments: vec![],
            },
            next_mode: next.map(|name| ModeId::new(name).expect("mode")),
        }
    }
    #[test]
    fn configured_transition_consumes_press_and_release() {
        let mut input = InputState::new(InputConfiguration {
            initial_mode: ModeId::new("normal").expect("mode"),
            modes: vec![
                mode(
                    "normal",
                    vec![binding("Super+Space", "select", Some("select"))],
                ),
                mode("select", vec![binding("Escape", "cancel", Some("normal"))]),
            ],
        })
        .expect("input");
        let outcome = input.press("Super+Space");
        assert!(outcome.consumed);
        assert_eq!(outcome.command.expect("command").name, "select");
        assert_eq!(input.current_mode().as_str(), "select");
        assert!(input.release("Super+Space"));
        assert!(!input.release("Super+Space"));
    }
    #[test]
    fn reloading_bindings_clears_consumed_input_and_recovers_missing_mode() {
        let normal = ModeId::new("normal").expect("mode");
        let mut input = InputState::new(InputConfiguration {
            initial_mode: normal.clone(),
            modes: vec![mode("normal", vec![binding("A", "one", None)])],
        })
        .expect("input");
        assert!(input.press("A").consumed);
        input
            .replace(InputConfiguration {
                initial_mode: normal,
                modes: vec![mode("normal", vec![binding("B", "two", None)])],
            })
            .expect("replace");
        assert!(!input.release("A"));
        assert!(!input.press("A").consumed);
        assert_eq!(input.press("B").command.expect("command").name, "two");
    }
}
