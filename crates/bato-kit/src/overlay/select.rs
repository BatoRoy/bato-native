//! `Select` (`Dropdown.tsx`): a value picker on [`dropdown`]. The trigger
//! shows the current option's label (or the placeholder, `Select…`); the
//! menu lists every option, the selected one with a `Check` icon (the
//! others have none, so their labels start further left: React's look).
//!
//! [`SelectOption`] is `types.ts`'s, shared with Combobox and MultiSelect.
//!
//! ```ignore
//! select(&on, &state.fruit, fruits(), Msg::Fruit)
//! ```

use iced::Length;

use super::dropdown::dropdown;
use crate::El;
use crate::ctx::Ctx;
use crate::icon::Ico;
use crate::menu::{MenuEntry, action};

/// `SelectOption<T>`: a value with its label, optionally disabled or with
/// an icon (Combobox rows).
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOption<T> {
    pub value: T,
    pub label: String,
    pub disabled: bool,
    pub icon: Option<Ico>,
}

/// `{ value, label }`.
pub fn opt<T>(value: T, label: impl Into<String>) -> SelectOption<T> {
    SelectOption {
        value,
        label: label.into(),
        disabled: false,
        icon: None,
    }
}

impl<T> SelectOption<T> {
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn icon(mut self, ico: Ico) -> Self {
        self.icon = Some(ico);
        self
    }
}

/// The label of the option holding `value`.
pub fn label_of<'o, T: PartialEq>(options: &'o [SelectOption<T>], value: &T) -> Option<&'o str> {
    options
        .iter()
        .find(|o| &o.value == value)
        .map(|o| o.label.as_str())
}

pub struct Select<'a, T, M> {
    cx: Ctx,
    value: T,
    options: Vec<SelectOption<T>>,
    on_change: Box<dyn Fn(T) -> M + 'a>,
    placeholder: String,
    align_right: bool,
    width: Length,
}

/// `<Select value options onChange />` on `cx` (the trigger's backdrop).
pub fn select<'a, T, M>(
    cx: &Ctx,
    value: T,
    options: Vec<SelectOption<T>>,
    on_change: impl Fn(T) -> M + 'a,
) -> Select<'a, T, M> {
    Select {
        cx: cx.clone(),
        value,
        options,
        on_change: Box::new(on_change),
        placeholder: "Select…".into(),
        align_right: false,
        width: Length::Shrink,
    }
}

impl<'a, T, M> Select<'a, T, M> {
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn align_right(mut self, on: bool) -> Self {
        self.align_right = on;
        self
    }
    pub fn width(mut self, w: impl Into<Length>) -> Self {
        self.width = w.into();
        self
    }
}

/// The menu entries: one action per option, `Check` on the selected one.
pub fn entries<T: PartialEq + Clone, M>(
    options: &[SelectOption<T>],
    value: &T,
    on_change: &dyn Fn(T) -> M,
) -> Vec<MenuEntry<M>> {
    options
        .iter()
        .map(|o| {
            let mut a = action(o.label.clone(), on_change(o.value.clone()));
            if &o.value == value {
                a = a.icon(Ico::Check);
            }
            a.into()
        })
        .collect()
}

impl<'a, T: PartialEq + Clone + 'a, M: Clone + 'a> From<Select<'a, T, M>> for El<'a, M> {
    fn from(s: Select<'a, T, M>) -> Self {
        let text = label_of(&s.options, &s.value)
            .map(str::to_string)
            .unwrap_or(s.placeholder);
        dropdown(&s.cx, text, entries(&s.options, &s.value, &*s.on_change))
            .align_right(s.align_right)
            .width(s.width)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_marks_the_selected_option() {
        let options = vec![opt("a", "Apple"), opt("b", "Banana")];
        let e = entries(&options, &"b", &|v| v);
        let icons: Vec<_> = e
            .iter()
            .map(|e| match e {
                MenuEntry::Action(a) => (a.label.clone(), a.icon, a.on_select),
                MenuEntry::Separator => unreachable!(),
            })
            .collect();
        assert_eq!(
            icons,
            vec![
                ("Apple".to_string(), None, "a"),
                ("Banana".to_string(), Some(Ico::Check), "b"),
            ]
        );
        assert_eq!(label_of(&options, &"a"), Some("Apple"));
        assert_eq!(label_of(&options, &"z"), None);
    }
}
