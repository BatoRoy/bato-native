//! Form controls: TextField, NumberInput, TextArea, TagsInput, OtpInput, MaskedInput,
//! CurrencyInput, RadioGroup, Slider, RangeSlider, VolumeControl, TimeInput (B1); Combobox,
//! MultiSelect, SearchInput, ColorPicker (B2); FileDropzone (B3); the Field scaffold lives in
//! `crate::field` (F2).
//!
//! Phase 3 batches add one file per component here and one `mod` line below.

pub mod file_dropzone;

pub use file_dropzone::{FileDropzone, file_dropzone};
