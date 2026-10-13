use ratatui::layout::Rect;

mod agent_select;
mod alert;
mod command_palette;
mod confirm;
mod fork;
mod help;
mod mcp;
mod model_select;
mod prompt_stash;
mod provider;
mod session_export;
mod session_list;
mod session_rename;
mod skill_list;
mod status;
mod subagent;
mod tag;
mod theme_list;
mod timeline;

const DIALOG_VIEWPORT_MARGIN_X: u16 = 2;
const DIALOG_VIEWPORT_MARGIN_Y: u16 = 1;
const DIALOG_INNER_PADDING_X: u16 = 1;
const DIALOG_INNER_PADDING_Y: u16 = 0;

pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let viewport = inset_rect(area, DIALOG_VIEWPORT_MARGIN_X, DIALOG_VIEWPORT_MARGIN_Y);
    if viewport.width == 0 || viewport.height == 0 {
        return area;
    }

    let dialog_width = width.min(viewport.width).max(1);
    let dialog_height = height.min(viewport.height).max(1);

    Rect {
        x: viewport.x + viewport.width.saturating_sub(dialog_width) / 2,
        y: viewport.y + viewport.height.saturating_sub(dialog_height) / 2,
        width: dialog_width,
        height: dialog_height,
    }
}

pub(super) fn dialog_inner(area: Rect) -> Rect {
    inset_rect(area, DIALOG_INNER_PADDING_X, DIALOG_INNER_PADDING_Y)
}

fn inset_rect(area: Rect, padding_x: u16, padding_y: u16) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }

    let twice_x = padding_x.saturating_mul(2);
    let twice_y = padding_y.saturating_mul(2);
    if area.width <= twice_x || area.height <= twice_y {
        return area;
    }

    Rect {
        x: area.x.saturating_add(padding_x),
        y: area.y.saturating_add(padding_y),
        width: area.width.saturating_sub(twice_x),
        height: area.height.saturating_sub(twice_y),
    }
}

pub use agent_select::{Agent, AgentSelectDialog};
pub use alert::AlertDialog;
pub use command_palette::CommandPalette;
pub use confirm::ConfirmDialog;
pub use fork::{ForkDialog, ForkEntry};
pub use help::HelpDialog;
pub use mcp::{McpDialog, McpItem};
pub use model_select::{Model, ModelSelectDialog};
pub use prompt_stash::{PromptStashDialog, StashItem};
pub use provider::{Provider, ProviderDialog, ProviderStatus};
pub use session_export::SessionExportDialog;
pub use session_list::{DeleteState as SessionDeleteState, SessionItem, SessionListDialog};
pub use session_rename::SessionRenameDialog;
pub use skill_list::SkillListDialog;
pub use status::{StatusDialog, StatusLine};
pub use subagent::{SubagentDialog, SubagentInfo, SubagentMessage};
pub use tag::{Tag, TagDialog};
pub use theme_list::{ThemeListDialog, ThemeOption};
pub use timeline::{TimelineDialog, TimelineEntry};
