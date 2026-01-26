//! Modal dialog system for termide.
//!
//! Provides themed modal dialogs for user interaction.
//! Uses termide-ui for base utilities and termide-theme for styling.

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use termide_theme::Theme;

// Re-export modal utilities from termide-ui
pub use termide_ui::{
    calculate_modal_width, centered_rect_with_size, max_item_width, max_line_width, ModalResult,
    ModalWidthConfig, TextInput as TextInputHandler,
};

pub mod base;
pub mod input_keys;
pub use base::{
    check_mouse_click, check_mouse_click_with_item_height, CursorNavigation, MouseClickResult,
};
pub use input_keys::{handle_input_key, InputKeyResult};
pub mod bookmark_add;
pub mod choice;
pub mod commit;
pub mod confirm;
pub mod conflict;
pub mod content_search;
pub mod directory_picker;
pub mod directory_switcher;
pub mod editable_select;
pub mod file_search;
pub mod info;
pub mod info_action;
pub mod input;
pub mod overwrite;
pub mod progress;
pub mod rename_pattern;
pub mod replace;
pub mod save_as;
pub mod search;
pub mod select;
pub mod sessions;

pub use bookmark_add::{BookmarkAddModal, BookmarkAddResult};
pub use choice::ChoiceModal;
pub use commit::CommitModal;
pub use confirm::ConfirmModal;
pub use conflict::{ConflictModal, ConflictResolution};
pub use content_search::{ContentSearchModal, ContentSearchResultItem};
pub use directory_picker::DirectoryPickerModal;
pub use directory_switcher::{DirectoryItem, DirectorySwitcherModal};
pub use editable_select::{EditableSelectModal, SelectOption};
pub use file_search::{FileSearchModal, SearchResultItem};
pub use info::InfoModal;
pub use info_action::{ActionButton, InfoActionModal, InfoActionResult};
pub use input::InputModal;
pub use overwrite::{OverwriteChoice, OverwriteModal};
pub use progress::ProgressModal;
pub use rename_pattern::RenamePatternModal;
pub use replace::{ReplaceAction, ReplaceModal, ReplaceModalResult};
pub use save_as::{SaveAsModal, SaveAsResult};
pub use search::{SearchAction, SearchModal, SearchModalResult};
pub use select::SelectModal;
pub use sessions::{SessionItem, SessionsModal};

/// Active modal window enum.
///
/// Contains all possible modal types in boxed form for dynamic dispatch.
#[derive(Debug)]
pub enum ActiveModal {
    /// Git commit modal
    Commit(Box<CommitModal>),
    /// Confirmation modal (Yes/No)
    Confirm(Box<ConfirmModal>),
    /// Choice modal with horizontal buttons
    Choice(Box<ChoiceModal>),
    /// Text input modal
    Input(Box<InputModal>),
    /// Selection modal (single selection)
    Select(Box<SelectModal>),
    /// File overwrite modal
    #[allow(dead_code)]
    Overwrite(Box<OverwriteModal>),
    /// File conflict resolution modal
    Conflict(Box<ConflictModal>),
    /// Information modal
    Info(Box<InfoModal>),
    /// Information modal with action buttons
    InfoAction(Box<InfoActionModal>),
    /// Rename pattern input modal
    RenamePattern(Box<RenamePatternModal>),
    /// Editable select modal (combobox)
    EditableSelect(Box<EditableSelectModal>),
    /// Interactive search modal
    Search(Box<SearchModal>),
    /// Interactive replace modal
    Replace(Box<ReplaceModal>),
    /// Sessions selection modal
    Sessions(Box<SessionsModal>),
    /// File search modal
    FileSearch(Box<FileSearchModal>),
    /// Content search modal
    ContentSearch(Box<ContentSearchModal>),
    /// Directory picker modal
    DirectoryPicker(Box<DirectoryPickerModal>),
    /// Save As modal with executable checkbox
    SaveAs(Box<SaveAsModal>),
    /// Directory switcher modal
    DirectorySwitcher(Box<DirectorySwitcherModal>),
    /// Bookmark add modal
    BookmarkAdd(Box<BookmarkAddModal>),
    /// Progress modal for long-running operations
    Progress(Box<ProgressModal>),
}

/// Trait for all modal windows.
///
/// This extends the base Modal concept with Theme support.
pub trait Modal {
    /// Modal window result type.
    type Result;

    /// Render the modal window with theme.
    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme);

    /// Handle keyboard event.
    /// Returns Some(result) if the modal window should close.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ModalResult<Self::Result>>>;

    /// Handle mouse event.
    /// Returns Some(result) if the modal window should close.
    fn handle_mouse(
        &mut self,
        _mouse: crossterm::event::MouseEvent,
        _modal_area: Rect,
    ) -> Result<Option<ModalResult<Self::Result>>> {
        Ok(None) // Default: do nothing
    }

    /// Handle paste event.
    /// Returns true if the modal handled the paste, false to pass to panel.
    fn handle_paste(&mut self, _text: &str) -> bool {
        false // Default: modals don't handle paste
    }
}
