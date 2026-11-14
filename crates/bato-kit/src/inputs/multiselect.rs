//! `MultiSelect` (`inputs/MultiSelect.tsx`): a searchable multi select.
//!
//! The trigger is a control box (`min-h-[2.375rem] px-2 py-1.5`, wrapping)
//! holding the selected options as chips (`bg-accent/15 text-accentBright
//! text-xs`, each with an × that removes it without opening the panel), or
//! the `muted` placeholder, and the turning chevron on the right. The
//! `matchWidth` panel has the autofocused search field and the option rows:
//! a checkbox look (16 px, accent fill and a stroke-3 check when selected),
//! `accentBright` text when selected, `hover:bg-text/6`; a click toggles
//! (up to `max`). React gives this list no keyboard navigation; neither
//! does the kit (Tab reaches the rows, Enter / Space toggle).
//!
//! ```ignore
//! multiselect(&on, &state.multi, fruits(), Msg::Multi).label("Multi-select")
//! ```

use std::rc::Rc;

use iced::widget::{Row, Space, container, row};
use iced::{Alignment, Length, Padding};

use super::combobox::{control_frame, filter, no_results, search_field, search_panel};
use crate::El;
use crate::ctx::Ctx;
use crate::field::field;
use crate::icon::{Ico, icon};
use crate::label::label;
use crate::overlay::dropdown::chevron;
use crate::overlay::popover::{local, popover};
use crate::overlay::select::SelectOption;
use crate::style::c;
use crate::surface::{BoxStyle, Status, surface};

pub struct MultiSelect<'a, T, M> {
    cx: Ctx,
    value: Vec<T>,
    options: Vec<SelectOption<T>>,
    on_change: Box<dyn Fn(Vec<T>) -> M + 'a>,
    placeholder: String,
    search_placeholder: String,
    label: Option<String>,
    hint: Option<String>,
    error: Option<String>,
    required: bool,
    disabled: bool,
    max: Option<usize>,
}

/// `<MultiSelect value options onChange />` on `cx` (the field's backdrop).
pub fn multiselect<'a, T: Clone, M>(
    cx: &Ctx,
    value: &[T],
    options: Vec<SelectOption<T>>,
    on_change: impl Fn(Vec<T>) -> M + 'a,
) -> MultiSelect<'a, T, M> {
    MultiSelect {
        cx: cx.clone(),
        value: value.to_vec(),
        options,
        on_change: Box::new(on_change),
        placeholder: "Select…".into(),
        search_placeholder: "Search…".into(),
        label: None,
        hint: None,
        error: None,
        required: false,
        disabled: false,
        max: None,
    }
}

impl<'a, T, M> MultiSelect<'a, T, M> {
    pub fn placeholder(mut self, s: impl Into<String>) -> Self {
        self.placeholder = s.into();
        self
    }
    pub fn search_placeholder(mut self, s: impl Into<String>) -> Self {
        self.search_placeholder = s.into();
        self
    }
    pub fn label(mut self, s: impl Into<String>) -> Self {
        self.label = Some(s.into());
        self
    }
    pub fn hint(mut self, s: impl Into<String>) -> Self {
        self.hint = Some(s.into());
        self
    }
    pub fn error(mut self, s: Option<&str>) -> Self {
        self.error = s.map(str::to_string);
        self
    }
    pub fn required(mut self, on: bool) -> Self {
        self.required = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// At most this many selected (further picks are ignored).
    pub fn max(mut self, n: usize) -> Self {
        self.max = Some(n);
        self
    }
}

/// `toggle(opt)`: the new value, or `None` when nothing changes (disabled,
/// or `max` reached).
pub fn toggle<T: PartialEq + Clone>(
    value: &[T],
    opt: &SelectOption<T>,
    max: Option<usize>,
) -> Option<Vec<T>> {
    if opt.disabled {
        return None;
    }
    if value.contains(&opt.value) {
        Some(value.iter().filter(|v| **v != opt.value).cloned().collect())
    } else if max.is_none_or(|m| value.len() < m) {
        let mut v = value.to_vec();
        v.push(opt.value.clone());
        Some(v)
    } else {
        None
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MultiState {
    pub open: bool,
    pub query: String,
}

#[derive(Debug, Clone)]
pub enum MultiEvent {
    Toggle,
    Close,
    Query(String),
    /// Toggle the option at this index of `options`.
    Pick(usize),
}

/// The checkbox look of a row (`w-4 h-4 rounded border`).
fn check_box<'a>(cx: &Ctx, on: bool) -> El<'a, MultiEvent> {
    let p = cx.p();
    let st = if on {
        BoxStyle::default()
            .bg(c(p.accent))
            .border(c(p.accent))
            .radius(4.0)
    } else {
        BoxStyle::default().border(c(p.control)).radius(4.0)
    };
    let inner: El<'a, MultiEvent> = if on {
        icon(Ico::Check, 11.0)
            .stroke_width(3.0)
            .color(c(p.accent_ink))
            .into()
    } else {
        Space::new().into()
    };
    surface(inner)
        .width(16)
        .height(16)
        .center()
        .fixed(st)
        .into()
}

/// A selected chip with its × (`pl-2 pr-1 py-0.5 gap-1 rounded-md`); `cx`
/// is the field fill it sits on.
fn chip<'a>(cx: &Ctx, text: &str, remove: MultiEvent) -> El<'a, MultiEvent> {
    let p = cx.p().clone();
    let bg = cx.blend(p.accent, 0.15);
    let x = surface(icon(Ico::X, 12.0))
        .focusable(false)
        .style(move |s| {
            BoxStyle::default().fg(c(if s == Status::Hover {
                p.text
            } else {
                p.accent_bright
            }))
        })
        .on_press(remove);
    surface(
        row![
            label(text.to_string(), 12.0, 16.0).color(c(cx.p().accent_bright)),
            x
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(Padding {
        top: 2.0,
        bottom: 2.0,
        left: 8.0,
        right: 4.0,
    })
    .fixed(BoxStyle::default().bg(c(bg)).radius(6.0))
    .into()
}

impl<'a, T, M> From<MultiSelect<'a, T, M>> for El<'a, M>
where
    T: PartialEq + Clone + 'a,
    M: Clone + 'a,
{
    fn from(ms: MultiSelect<'a, T, M>) -> Self {
        let MultiSelect {
            cx,
            value,
            options,
            on_change,
            placeholder,
            search_placeholder,
            label: lab,
            hint,
            error,
            required,
            disabled,
            max,
        } = ms;
        let options = Rc::new(options);
        let opts = options.clone();
        let value = Rc::new(value);
        let val = value.clone();
        let fcx = cx.clone();
        let has_error = error.is_some();
        let control = local(
            move |s: &MultiState| {
                let p = cx.p();
                // The chips sit on the field fill (`bg-app-bg/50` over the card).
                let inside = cx.on(cx.recessed());
                let selected: Vec<usize> = (0..opts.len())
                    .filter(|&i| val.contains(&opts[i].value))
                    .collect();
                let mut chips: Vec<El<'_, MultiEvent>> = Vec::new();
                if selected.is_empty() {
                    chips.push(
                        container(label(placeholder.clone(), 14.0, 20.0).color(c(p.muted)))
                            .padding(Padding::from([0.0, 4.0]))
                            .into(),
                    );
                }
                for &i in &selected {
                    chips.push(chip(&inside, &opts[i].label, MultiEvent::Pick(i)));
                }
                let content = row![
                    // `min-h-[2.375rem]`: 24 px inside the padding and border
                    // (1 px wide: iced drops zero-width children; the left
                    // padding gives that pixel back).
                    Space::new().width(1).height(24),
                    Row::with_children(chips)
                        .spacing(6)
                        .align_y(Alignment::Center)
                        .wrap(),
                    Space::new().width(Length::Fill),
                    chevron(s.open, 14.0).color(c(p.muted)),
                ]
                .align_y(Alignment::Center);
                let trigger = control_frame(
                    &cx,
                    surface(content)
                        .padding(Padding {
                            top: 7.0,
                            bottom: 7.0,
                            left: 8.0,
                            right: 9.0,
                        })
                        .width(Length::Fill)
                        .disabled(disabled)
                        .on_press(MultiEvent::Toggle),
                )
                .error(has_error)
                .disabled(disabled)
                .blur_on_panel_press(true);
                let panel = s.open.then(|| {
                    let on = cx.on_card();
                    let p = on.p().clone();
                    let f = filter(&opts, &s.query);
                    let rows: Vec<El<'_, MultiEvent>> = if f.is_empty() {
                        vec![no_results(&on)]
                    } else {
                        f.into_iter()
                            .map(|i| {
                                let o = &opts[i];
                                let sel = val.contains(&o.value);
                                let hover = c(on.wash(0.06));
                                let fg = c(if sel { p.accent_bright } else { p.subtext });
                                let r = surface(
                                    row![
                                        check_box(&on, sel),
                                        label(o.label.clone(), 14.0, 20.0)
                                            .ellipsis()
                                            .width(Length::Fill),
                                    ]
                                    .spacing(10)
                                    .align_y(Alignment::Center),
                                )
                                .padding(Padding::from([6.0, 12.0]))
                                .width(Length::Fill)
                                .style(move |st| {
                                    let b = BoxStyle::default().fg(fg);
                                    if st == Status::Hover { b.bg(hover) } else { b }
                                })
                                .disabled(o.disabled)
                                .on_press(MultiEvent::Pick(i));
                                if o.disabled {
                                    crate::anim::opacity(0.4, r).into()
                                } else {
                                    r.into()
                                }
                            })
                            .collect()
                    };
                    let search = crate::focus::autofocus(search_field(
                        &on,
                        &search_placeholder,
                        &s.query,
                        MultiEvent::Query,
                    ));
                    search_panel(&cx, search.into(), rows)
                });
                popover(trigger, panel)
                    .match_width(true)
                    .on_dismiss(|_| MultiEvent::Close)
                    .into()
            },
            move |s: &mut MultiState, e| match e {
                MultiEvent::Toggle => {
                    s.open = !s.open;
                    None
                }
                MultiEvent::Close => {
                    s.open = false;
                    None
                }
                MultiEvent::Query(q) => {
                    s.query = q;
                    None
                }
                MultiEvent::Pick(i) => toggle(&value, &options[i], max).map(&on_change),
            },
        );
        let mut f = field(&fcx, control)
            .required(required)
            .error(error.as_deref());
        if let Some(l) = lab {
            f = f.label(l);
        }
        if let Some(h) = hint {
            f = f.hint(h);
        }
        f.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::select::opt;

    #[test]
    fn toggle_adds_removes_and_honours_max() {
        let a = opt("a", "A");
        let b = opt("b", "B");
        let c = opt("c", "C").disabled(true);
        assert_eq!(toggle(&["a"], &b, None), Some(vec!["a", "b"]));
        assert_eq!(toggle(&["a", "b"], &a, None), Some(vec!["b"]));
        assert_eq!(toggle(&["a"], &b, Some(1)), None, "max reached");
        assert_eq!(
            toggle(&["a"], &a, Some(1)),
            Some(vec![]),
            "removing always works"
        );
        assert_eq!(toggle(&[], &c, None), None, "disabled");
    }
}
