use eframe::egui::{InputState, Key, Modifiers, Ui, UiBuilder};

#[derive(Default, Clone, PartialEq)]
pub struct ScrollValue {
    pub index: usize,
    pub pixel_offset: f32,
}

pub fn clamp_scroller(value: &mut ScrollValue, len: usize) {
    if value.index == 0 && value.pixel_offset < 0.0 {
        value.pixel_offset = 0.0;
    }
    if value.index >= len && value.pixel_offset > 0.0 {
        value.pixel_offset = 0.0;
    }
}

pub fn scroller_mouse_input(input: &mut InputState, value: &mut ScrollValue) {
    let scroll_mult = -4.0; // inverted?
    value.pixel_offset += input.smooth_scroll_delta.y * scroll_mult;
    input.smooth_scroll_delta.y = 0.0;
}

pub fn scroller_key_input(
    input: &mut InputState,
    value: &mut ScrollValue,
    len: usize,
    ui_height: f32,
) {
    if input.consume_key(Modifiers::NONE, Key::J) {
        value.pixel_offset += 20.0; // TODO: how to get line height?
    }
    if input.consume_key(Modifiers::NONE, Key::K) {
        value.pixel_offset -= 20.0; // TODO: how to get line height?
    }
    if input.consume_key(Modifiers::NONE, Key::D)
        || input.consume_key(Modifiers::CTRL, Key::D)
        || input.consume_key(Modifiers::NONE, Key::PageDown)
    {
        value.pixel_offset += ui_height / 2.0;
    }
    if input.consume_key(Modifiers::NONE, Key::U)
        || input.consume_key(Modifiers::CTRL, Key::U)
        || input.consume_key(Modifiers::NONE, Key::PageUp)
    {
        value.pixel_offset -= ui_height / 2.0;
    }
    if input.consume_key(Modifiers::NONE, Key::End) || input.consume_key(Modifiers::SHIFT, Key::G) {
        value.index = len - 1;
    }
    if input.consume_key(Modifiers::NONE, Key::Home) || input.consume_key(Modifiers::NONE, Key::G) {
        value.index = 0;
    }
}

pub fn big_scroller(
    ui: &mut Ui,
    value: &mut ScrollValue,
    len: usize,
    mut draw: impl FnMut(&mut Ui, usize),
) {
    ui.scope(|ui| {
        let max_rect = ui.max_rect();
        let absolute_begin = ui.next_widget_position().y;
        ui.add_space(-value.pixel_offset);
        ui.skip_ahead_auto_ids(value.index);
        for index in value.index..len {
            let begin = ui.next_widget_position().y;
            draw(ui, index);
            let end = ui.next_widget_position().y;
            if end > max_rect.bottom() {
                break;
            }
            if end < absolute_begin && value.index < len {
                value.index += 1;
                let size = end - begin;
                value.pixel_offset -= size;
            }
        }
    });

    // this messes up the drawing of the main content, idk why, so put it after
    if value.pixel_offset < 0.0 && value.index > 0 {
        ui.scope_builder(UiBuilder::new().sizing_pass().invisible(), |ui| {
            while value.pixel_offset < 0.0 && value.index > 0 {
                let begin = ui.next_widget_position().y;
                draw(ui, value.index - 1);
                let end = ui.next_widget_position().y;
                value.index -= 1;
                value.pixel_offset += end - begin;
            }
        });
    }
}
