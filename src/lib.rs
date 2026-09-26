//! steamship's internals. The command line is the supported interface; these modules are public
//! so that the tests and fuzz targets can reach them, and may change in any release.

pub mod check;
pub mod elf;
pub mod pattern;
pub mod scripts;
#[cfg(unix)]
pub mod unix;
pub mod vdf;
