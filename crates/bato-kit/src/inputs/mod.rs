//! Form controls: TextField, NumberInput, TextArea, TagsInput, OtpInput, MaskedInput,
//! CurrencyInput, RadioGroup, Slider, RangeSlider, VolumeControl, TimeInput (B1); Combobox,
//! MultiSelect, SearchInput, ColorPicker (B2); FileDropzone (B3); the Field scaffold lives in
//! `crate::field` (F2).
//!
//! Phase 3 batches add one file per component here and one `mod` line below.

// B1 basic inputs.
pub mod file_dropzone;
pub mod format;
pub mod frame;
pub mod number_input;
pub mod otp_input;
pub mod slider;
pub mod tags_input;
pub mod text_area;
pub mod text_field;
pub mod time_input;
pub mod toggles;

pub use file_dropzone::{FileDropzone, file_dropzone};
