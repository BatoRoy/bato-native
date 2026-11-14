//! The **Inputs** tab: its cards in page order (docs/CARDS.md). One file
//! per card; a batch edits only its own card files and its line here.

crate::showcase::tab_cards! {
    text_fields: TextFields => "text-fields",
    specialized: Specialized => "specialized",
    selects: Selects => "selects",
    sliders_color: SlidersColor => "sliders-color",
    time_input: TimeInput => "time-input",
    search: Search => "search",
    code_editor: CodeEditor => "code-editor",
    toggles_upload: TogglesUpload => "toggles-upload",
}
