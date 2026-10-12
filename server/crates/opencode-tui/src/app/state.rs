#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AppState {
    #[default]
    Running,
    Exiting,
    PromptFocused,
    DialogOpen,
    CommandPalette,
}
