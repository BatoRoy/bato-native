//! Feedback and layout atoms (B4): Skeleton, Progress, CircularProgress, Alert, Breadcrumbs,
//! Avatar, Stepper, Accordion, Divider, AppShell, PanelGroup.
//!
//! Phase 3 batches add one file per component here and one `mod` line below.
//! The layout half (Divider, Stack/Center/Container, AppShell, PanelGroup,
//! the animated `collapse`) lives in [`crate::layout`].

mod accordion;
mod alert;
mod avatar;
mod breadcrumbs;
mod progress;
mod skeleton;
mod stepper;

pub use accordion::{accordion, body_text, collapsible, toggle};
pub use alert::{Alert, alert, tone_icon};
pub use avatar::{AvatarSize, avatar, avatar_group, initials};
pub use breadcrumbs::{Crumb, breadcrumbs};
pub use progress::{CircularProgress, Progress, circular_progress, progress, ring_svg};
pub use skeleton::{Skeleton, skeleton};
pub use stepper::{Step, StepState, Stepper, stepper};
