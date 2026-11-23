use super::{handle, open, window_closed};
use crate::{
    player::{controller, view::PlayerView},
    ui::Pal,
};
use gpui::{Modifiers, TestAppContext, VisualTestContext, point, px};

#[gpui::test]
fn closing_pip_preserves_playing_video(cx: &mut TestAppContext) {
    check_pip_handoff(cx, false);
}

#[gpui::test]
fn closing_pip_preserves_paused_video(cx: &mut TestAppContext) {
    check_pip_handoff(cx, true);
}

fn check_pip_handoff(cx: &mut TestAppContext, paused_in_pip: bool) {
    let main_player = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(true, 0x3ea6ff));
        controller::init(cx);
        cx.on_window_closed(|cx, id| window_closed(id, cx)).detach();
        let player = controller::player(cx);
        player.update(cx, |player, _| {
            player.duration = 160.0;
            player.seek_to(34.0);
            player.paused = !paused_in_pip;
        });
        player
    });
    let main_window = cx.add_window(|_, cx| PlayerView::new(main_player.clone(), false, cx));

    cx.update(open);
    let panel = cx.read(handle).expect("PiP window opened");
    let mut pip = VisualTestContext::from_window(panel, cx);
    let panel_size = pip.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.viewport_size()
    });
    let centre = point(panel_size.width / 2.0, panel_size.height / 2.0);
    pip.simulate_mouse_move(centre, None, Modifiers::default());
    pip.simulate_click(centre, Modifiers::default());
    assert_eq!(
        main_player.read_with(cx, |player, _| player.paused),
        paused_in_pip
    );

    // Stand in for progress delivered by mpv while the panel owns the video surface.
    main_player.update(cx, |player, _| player.seek_to(83.25));
    let close = point(panel_size.width - px(16.0), px(16.0));
    pip.simulate_mouse_move(close, None, Modifiers::default());
    pip.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    pip.simulate_click(close, Modifiers::default());

    assert!(
        cx.read(handle).is_none(),
        "the closed panel must release the main player"
    );
    assert!(
        main_window.root(cx).is_ok(),
        "the main window must remain open"
    );
    assert_eq!(
        main_player.read_with(cx, |player, _| (player.time, player.paused)),
        (83.25, paused_in_pip)
    );
}
