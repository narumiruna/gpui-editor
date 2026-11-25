use std::time::Duration;

use gpui::{
    Context, Div, Entity, FocusHandle, Stateful, Window, div, img, prelude::*, px, rgb, rgba,
};

use super::{Shell, toolbar};
use bridge::{Attachment, Bridge, Event, Target};
pub(crate) use picker::Picker;
use text_field::TextField;

mod bridge;
mod picker;
mod picture;
mod text_field;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Developer {
    bridge: Option<Bridge>,
    generation: u64,
    partner: Option<Partner>,
    preferences: bool,
    picking: bool,
    return_focus: Option<FocusHandle>,
    draft: Option<Draft>,
    notice: Option<String>,
}

struct Partner {
    id: u64,
    project: String,
    session: String,
    code: String,
    state: PairState,
}

#[derive(PartialEq, Eq)]
enum PairState {
    Requested,
    Confirming,
    Connected,
}

struct Draft {
    target: Target,
    text: Entity<TextField>,
    picture: Option<picture::Picture>,
    loading: bool,
    capturing: bool,
    delivery: Option<u32>,
    notice: Option<String>,
}

impl Developer {
    pub(super) fn blocks_document_actions(&self) -> bool {
        self.preferences
            || self.picking
            || self.draft.is_some()
            || self
                .partner
                .as_ref()
                .is_some_and(|p| p.state == PairState::Requested)
    }

    pub(super) fn record(&self, event: &'static str) {
        if let Some(bridge) = &self.bridge {
            bridge.record(event);
        }
    }

    pub(super) fn picker(&self) -> Option<Picker> {
        self.picking.then(Picker::default)
    }

    fn connected(&self) -> bool {
        self.partner
            .as_ref()
            .is_some_and(|p| p.state == PairState::Connected)
    }
}

impl Shell {
    pub(super) fn developer_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some()
            || self.developer.blocks_document_actions()
            || self.is_composing(cx)
        {
            return;
        }
        self.developer.return_focus = window.focused(cx);
        self.developer.preferences = true;
        window.focus(&self.focus);
        cx.notify();
    }

    fn is_composing(&self, cx: &Context<Self>) -> bool {
        self.current
            .as_ref()
            .is_some_and(|doc| doc.editor.read(cx).is_composing())
    }

    fn toggle_developer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dev = &mut self.developer;
        dev.generation += 1;
        dev.notice = None;
        if dev.bridge.take().is_some() {
            dev.partner = None;
            dev.draft = None;
            dev.picking = false;
        } else {
            match Bridge::start() {
                Ok(bridge) => {
                    bridge.record(concat!(
                        "Developer Mode enabled · GPUI Editor ",
                        env!("CARGO_PKG_VERSION")
                    ));
                    dev.bridge = Some(bridge);
                    let generation = dev.generation;
                    cx.spawn_in(window, async move |shell, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(50))
                                .await;
                            let live = shell.update_in(cx, |shell, window, cx| {
                                if shell.developer.generation != generation {
                                    return false;
                                }
                                while let Some(event) =
                                    shell.developer.bridge.as_ref().and_then(Bridge::try_event)
                                {
                                    shell.developer_event(event, window, cx);
                                }
                                true
                            });
                            if !matches!(live, Ok(true)) {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                Err(_) => dev.notice = Some("Could not start the local pairing service.".into()),
            }
        }
        cx.notify();
    }

    fn developer_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Pair {
                id,
                project,
                session,
                code,
            } => {
                if self.active.is_some()
                    || self.is_composing(cx)
                    || self.developer.partner.is_some()
                    || self
                        .developer
                        .draft
                        .as_ref()
                        .is_some_and(|draft| draft.capturing)
                {
                    if let Some(bridge) = &self.developer.bridge {
                        bridge.decide(id, false);
                    }
                    return;
                }
                if !self.developer.blocks_document_actions() {
                    self.developer.return_focus = window.focused(cx);
                }
                self.developer.record("Pi pairing requested");
                self.developer.partner = Some(Partner {
                    id,
                    project,
                    session,
                    code,
                    state: PairState::Requested,
                });
                window.activate_window();
                window.focus(&self.focus);
            }
            Event::Paired { id } => {
                if let Some(partner) = &mut self.developer.partner
                    && partner.id == id
                    && partner.state == PairState::Confirming
                {
                    partner.state = PairState::Connected;
                    self.developer.record("Pi session paired");
                }
            }
            Event::Disconnected { id } => {
                if self.developer.partner.as_ref().is_none_or(|p| p.id != id) {
                    return;
                }
                self.developer.partner = None;
                self.developer.record("Pi session disconnected");
                if let Some(draft) = &mut self.developer.draft {
                    let uncertain = draft.delivery.take().is_some();
                    if uncertain {
                        draft
                            .text
                            .update(cx, |text, cx| text.set_editable(true, cx));
                    }
                    draft.notice = Some(if uncertain {
                        "Disconnected before Pi confirmed receipt. Check Pi before sending again."
                    } else {
                        "Pi disconnected. Run /gpui-pair again to send this draft."
                    }.into());
                }
                self.developer.picking = false;
                self.resume_developer_focus(window, cx);
            }
            Event::Receipt {
                id,
                accepted,
                message,
            } => {
                let Some(draft) = &mut self.developer.draft else {
                    return;
                };
                if draft.delivery != Some(id) {
                    return;
                }
                draft.delivery = None;
                if accepted {
                    self.developer.draft = None;
                    self.developer.notice = Some("Feedback handed to Pi.".into());
                    self.resume_developer_focus(window, cx);
                } else {
                    draft
                        .text
                        .update(cx, |text, cx| text.set_editable(true, cx));
                    draft.notice =
                        Some(message.unwrap_or_else(|| "Pi did not accept the feedback.".into()));
                }
            }
        }
        cx.notify();
    }

    fn decide_pair(&mut self, approved: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(partner) = &mut self.developer.partner
            && partner.state == PairState::Requested
            && let Some(bridge) = &self.developer.bridge
        {
            bridge.decide(partner.id, approved);
            if approved {
                partner.state = PairState::Confirming;
            } else {
                self.developer.partner = None;
                self.developer.record("Pi pairing declined");
            }
            self.resume_developer_focus(window, cx);
            cx.notify();
        }
    }

    fn unpair(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(bridge), Some(partner)) = (&self.developer.bridge, &self.developer.partner) {
            let id = partner.id;
            bridge.unpair();
            self.developer_event(Event::Disconnected { id }, window, cx);
        }
    }

    fn start_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.developer.connected()
            || self.developer.blocks_document_actions()
            || self.is_composing(cx)
        {
            return;
        }
        self.developer.notice = None;
        self.developer.return_focus = window.focused(cx);
        self.developer.picking = true;
        window.focus(&self.focus);
        cx.notify();
    }

    fn choose_target(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if !self.developer.picking || !self.developer.connected() {
            return;
        }
        self.developer.picking = false;
        self.developer.draft = Some(Draft {
            target,
            text: cx.new(TextField::new),
            picture: None,
            loading: false,
            capturing: false,
            delivery: None,
            notice: None,
        });
        self.resume_developer_focus(window, cx);
        cx.notify();
    }

    pub(super) fn resume_developer_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        if self.developer.preferences
            || self.developer.picking
            || self
                .developer
                .partner
                .as_ref()
                .is_some_and(|p| p.state == PairState::Requested)
        {
            window.focus(&self.focus);
        } else if let Some(draft) = &self.developer.draft {
            draft.text.read(cx).focus(window);
        } else if let Some(focus) = self.developer.return_focus.take() {
            window.focus(&focus);
        }
    }

    pub(super) fn cancel_developer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .developer
            .partner
            .as_ref()
            .is_some_and(|p| p.state == PairState::Requested)
        {
            self.decide_pair(false, window, cx);
            return;
        }
        if self
            .developer
            .draft
            .as_ref()
            .is_some_and(|draft| draft.delivery.is_some() || draft.capturing)
        {
            return;
        }
        self.developer.preferences = false;
        self.developer.picking = false;
        self.developer.draft = None;
        self.resume_developer_focus(window, cx);
        cx.notify();
    }

    fn add_picture(&mut self, capture: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.developer.draft else {
            return;
        };
        if draft.loading || draft.delivery.is_some() {
            return;
        }
        if capture && draft.text.read(cx).is_composing() {
            draft.notice =
                Some("Finish or cancel text composition before capturing the window.".into());
            cx.notify();
            return;
        }
        let input = if capture {
            picture::window_id(window).map(PictureInput::Window)
        } else {
            picture::clipboard(cx).map(PictureInput::Clipboard)
        };
        let input = match input {
            Ok(input) => input,
            Err(error) => {
                draft.notice = Some(error);
                cx.notify();
                return;
            }
        };
        draft.loading = true;
        draft.capturing = capture;
        draft.notice = None;
        let text = draft.text.clone();
        cx.notify();
        cx.spawn_in(window, async move |shell, cx| {
            if capture {
                // Let GPUI paint the window without the feedback sheet first.
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let still_capturing = shell.update_in(cx, |shell, _, _| {
                    shell.active.is_none()
                        && shell
                            .developer
                            .draft
                            .as_ref()
                            .is_some_and(|draft| draft.text == text && draft.capturing)
                });
                if !matches!(still_capturing, Ok(true)) {
                    let _ = shell.update_in(cx, |shell, _, cx| {
                        if let Some(draft) = &mut shell.developer.draft
                            && draft.text == text
                        {
                            draft.loading = false;
                            draft.capturing = false;
                            cx.notify();
                        }
                    });
                    return;
                }
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    match input {
                        PictureInput::Window(id) => picture::capture(id),
                        PictureInput::Clipboard(image) => picture::from_clipboard(image),
                    }
                })
                .await;
            shell
                .update_in(cx, |shell, _, cx| {
                    if let Some(draft) = &mut shell.developer.draft
                        && draft.text == text
                    {
                        draft.loading = false;
                        draft.capturing = false;
                        match result {
                            Ok(picture) => draft.picture = Some(picture),
                            Err(error) => draft.notice = Some(error),
                        }
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    fn send_feedback(&mut self, cx: &mut Context<Self>) {
        let connected = self.developer.connected();
        let Some(draft) = &mut self.developer.draft else {
            return;
        };
        if !connected || draft.delivery.is_some() || draft.loading {
            return;
        }
        let input = draft.text.read(cx);
        let text = input.text();
        if input.is_composing() {
            draft.notice = Some("Finish or cancel text composition before sending.".into());
        } else if text.trim().is_empty() && draft.picture.is_none() {
            draft.notice = Some("Enter text or attach an image first.".into());
        } else if let Some(bridge) = &self.developer.bridge {
            let image = draft.picture.as_ref().map(|picture| Attachment {
                mime_type: "image/png".into(),
                data: picture.data.clone(),
            });
            match bridge.send_feedback(draft.target.clone(), text.into(), image) {
                Ok(id) => {
                    draft.delivery = Some(id);
                    draft
                        .text
                        .update(cx, |text, cx| text.set_editable(false, cx));
                    draft.notice = Some("Handing feedback to Pi…".into());
                }
                Err(_) => draft.notice = Some("Could not send. The draft is still here.".into()),
            }
        }
        cx.notify();
    }

    pub(super) fn developer_icon(
        &self,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Option<Stateful<Div>> {
        self.developer.bridge.as_ref()?;
        let connected = self.developer.connected();
        let ready = connected && enabled;
        let color = rgb(if ready { 0x21478e } else { 0x475569 });
        let reason = if connected {
            "Select an element for Pi"
        } else {
            "Not paired. Run /gpui-pair in Pi."
        };
        Some(
            div()
                .id("developer-picker")
                .debug_selector(|| "developer-picker".into())
                .m_1()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .min_h(px(30.))
                .rounded_md()
                .px_2()
                .bg(rgb(if ready { 0xe4eeff } else { 0xf2f5fa }))
                .text_color(color)
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(toolbar::icon("Developer target", 18.).text_color(color))
                .child("Select element")
                .tooltip(move |_, cx| cx.new(|_| toolbar::Tooltip(reason)).into())
                .when(ready, |el| {
                    el.cursor_pointer()
                        .hover(|style| style.bg(rgb(0xd3e3ff)))
                        .on_click(
                            cx.listener(|shell, _, window, cx| shell.start_picker(window, cx)),
                        )
                }),
        )
    }

    pub(super) fn developer_status(&self) -> Option<&str> {
        if self.developer.connected() {
            self.developer.notice.as_deref()
        } else {
            None
        }
    }

    pub(super) fn developer_layer(
        &self,
        picker: Option<Picker>,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let dev = &self.developer;
        if let Some(picker) = picker {
            return Some(
                div().absolute().inset_0().child(picker.highlight()).child(
                    div()
                        .id("element-picker-layer")
                        .absolute()
                        .inset_0()
                        .cursor(gpui::CursorStyle::Crosshair)
                        .block_mouse_except_scroll()
                        .on_mouse_move(|_, window, _| window.refresh())
                        .on_click(cx.listener(
                            move |shell, event: &gpui::ClickEvent, window, cx| {
                                if let Some(target) = picker.target_at(event.position()) {
                                    shell.choose_target(target, window, cx);
                                }
                            },
                        )),
                ),
            );
        }
        if let Some(partner) = &dev.partner
            && partner.state == PairState::Requested
        {
            return Some(sheet(panel("Pair with Pi?")
                .child(format!("Project: {}", partner.project))
                .child(format!("Session: {}", partner.session))
                .child(format!("Confirm this code matches Pi: {}", partner.code))
                .child("This session can receive feedback you send and read recent editor diagnostics.")
                .child(row()
                    .child(button("Decline", true).on_click(cx.listener(|shell, _, window, cx| shell.decide_pair(false, window, cx))))
                    .child(button("Pair", true).on_click(cx.listener(|shell, _, window, cx| shell.decide_pair(true, window, cx)))))));
        }
        if dev.preferences {
            let enabled = dev.bridge.is_some();
            return Some(sheet(
                panel("Preferences")
                    .child(
                        button(
                            if enabled {
                                "Developer Mode: On"
                            } else {
                                "Developer Mode: Off"
                            },
                            true,
                        )
                        .on_click(
                            cx.listener(|shell, _, window, cx| shell.toggle_developer(window, cx)),
                        ),
                    )
                    .when(enabled, |el| {
                        el.child(if dev.connected() {
                            "Pi is paired."
                        } else {
                            "Run /gpui-pair in Pi to connect to this window."
                        })
                    })
                    .when_some(dev.notice.clone(), |el, notice| el.child(notice))
                    .child(
                        row()
                            .when(dev.connected(), |el| {
                                el.child(button("Unpair Pi", true).on_click(
                                    cx.listener(|shell, _, window, cx| shell.unpair(window, cx)),
                                ))
                            })
                            .child(button("Done", true).on_click(cx.listener(
                                |shell, _, window, cx| shell.cancel_developer(window, cx),
                            ))),
                    ),
            ));
        }
        let draft = dev.draft.as_ref()?;
        if draft.capturing {
            return Some(div().absolute().inset_0().occlude());
        }
        let busy = draft.delivery.is_some();
        Some(sheet(panel("Send feedback to Pi")
            .child(format!("Element: {}", draft.target.label))
            .child(dev.partner.as_ref().filter(|p| p.state == PairState::Connected)
                .map_or_else(|| "Pi is not paired.".into(), |p| format!("Pi: {} · {}", p.project, p.session)))
            .child(draft.text.clone())
            .child("Review attachments before sending. Feedback may be saved in Pi or sent to its model provider.")
            .when_some(draft.picture.as_ref().map(|p| p.preview.clone()), |el, preview| el
                .child(img(preview).w(px(220.)).h(px(100.)).object_fit(gpui::ObjectFit::Contain)))
            .when_some(draft.notice.clone(), |el, notice| el.child(notice))
            .child(row()
                .child(button("Capture Window", !busy && !draft.loading).on_click(cx.listener(|shell, _, window, cx| shell.add_picture(true, window, cx))))
                .child(button("Paste Image", !busy && !draft.loading).on_click(cx.listener(|shell, _, window, cx| shell.add_picture(false, window, cx))))
                .when(draft.picture.is_some() && !busy && !draft.loading, |el| el.child(button("Remove Image", true)
                    .on_click(cx.listener(|shell, _, _, cx| {
                        if let Some(draft) = &mut shell.developer.draft { draft.picture = None; }
                        cx.notify();
                    })))))
            .child(row()
                .child(button("Cancel", !busy).on_click(cx.listener(|shell, _, window, cx| shell.cancel_developer(window, cx))))
                .child(button("Send to Pi", dev.connected() && !busy && !draft.loading)
                    .on_click(cx.listener(|shell, _, _, cx| shell.send_feedback(cx)))))))
    }
}

// Use the platform clipboard only on the GPUI thread; encoding and native capture run off it.
enum PictureInput {
    Window(u32),
    Clipboard(std::sync::Arc<gpui::Image>),
}

fn sheet(content: impl IntoElement) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p_3()
        .bg(rgba(0x0f172a44))
        .child(content)
}

fn panel(title: &'static str) -> Stateful<Div> {
    div()
        .id(title)
        .debug_selector(move || title.into())
        .w_full()
        .max_w(px(540.))
        .max_h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_md()
        .bg(rgb(0xffffff))
        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
}

fn row() -> Div {
    div().flex().items_center().flex_wrap().gap_2()
}

fn button(label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(label)
        .debug_selector(move || label.into())
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgb(if enabled { 0xe4eeff } else { 0xf2f5fa }))
        .text_color(rgb(if enabled { 0x172336 } else { 0x64748b }))
        .when(enabled, |el| {
            el.cursor_pointer().hover(|el| el.bg(rgb(0xd3e3ff)))
        })
        .child(label)
}
