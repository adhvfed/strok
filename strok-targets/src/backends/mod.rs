//! Framework backends — co-equal renderers of the neutral `UiDoc`.

pub mod jsx;
pub mod react;
pub mod solid;
pub mod vanilla;

pub use react::ReactBackend;
pub use solid::SolidBackend;
pub use vanilla::VanillaBackend;
