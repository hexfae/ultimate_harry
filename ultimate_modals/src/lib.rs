//! This crate exists because both `ultimate_character` and `ultimate_harry` need `EditModal` and `SecondEditModal`, but defining them in `ultimate_harry` and importing it in `ultimate_character` (which `ultimate_harry` depends on) creates a cyclic dependency.

mod modals;

pub use modals::CreateCharacterModal;
pub use modals::EditCharacterModal;
pub use modals::EditMessageModal;
pub use modals::SecondCreateCharacterModal;
pub use modals::SecondEditCharacterModal;
