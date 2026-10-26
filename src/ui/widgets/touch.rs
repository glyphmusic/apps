use super::*;

/// Claim direct manipulation before GPUI turns a finger movement into scrolling.
/// Ownership survives redraws and leaving the control; cancellation also releases it.
pub fn touch_drag(
    window: &mut Window,
    key: impl Into<SharedString>,
    hitbox: gpui::Hitbox,
    on: impl Fn(&gpui::TouchDragEvent, &mut Window, &mut gpui::App) + 'static,
) {
    let key = key.into();
    let win = window.window_handle().window_id();
    window.on_mouse_event(move |e: &gpui::TouchDragEvent, phase, window, cx| {
        if phase != gpui::DispatchPhase::Bubble {
            return;
        }
        let started = e.phase == gpui::TouchPhase::Started;
        if started {
            if window.default_prevented() || !hitbox.is_hovered_at(e.start_position, window) {
                return;
            }
            UI_RT.with(|rt| rt.borrow_mut().win_mut(win).touch_drag = Some(key.clone()));
            window.prevent_default();
        } else if !UI_RT.with(|rt| rt.borrow().win(win).is_some_and(|w| w.touch_drag.as_ref() == Some(&key))) {
            return;
        }
        if matches!(e.phase, gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled) {
            UI_RT.with(|rt| rt.borrow_mut().win_mut(win).touch_drag = None);
        }
        cx.stop_propagation();
        on(e, window, cx);
    });
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{point, PlatformInput, Render, TestAppContext, TouchEvent, TouchId, TouchPhase};
    use std::cell::RefCell;

    struct Control(Rc<RefCell<Vec<(TouchPhase, f32)>>>);

    impl Render for Control {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            begin_window_frame(window);
            let events = self.0.clone();
            let view = cx.entity().downgrade();
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    wheel_control(window, hitbox.clone());
                    let events = events.clone();
                    let view = view.clone();
                    touch_drag(window, "test-control", hitbox, move |e, _, cx| {
                        events.borrow_mut().push((e.phase, f32::from(e.position.x)));
                        view.update(cx, |_, cx| cx.notify()).ok();
                    });
                },
            ).size(px(100.0))
        }
    }

    fn send(window: gpui::WindowHandle<Control>, cx: &mut TestAppContext, phase: TouchPhase, x: f32) {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(PlatformInput::Touch(TouchEvent {
                id: TouchId(1), phase, position: point(px(x), px(10.0)), predicted_position: None, force: None,
            }), cx);
        }).unwrap();
        // Real controls redraw after every movement. Ownership must survive this.
        cx.executor().run_until_parked();
    }

    #[gpui::test]
    fn drag_survives_slop_redraw_and_leaving_control(cx: &mut TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({ let events = events.clone(); move |_, _| Control(events) });
        send(window, cx, TouchPhase::Started, 10.0);
        send(window, cx, TouchPhase::Moved, 30.0);
        send(window, cx, TouchPhase::Moved, 250.0);
        send(window, cx, TouchPhase::Ended, 300.0);
        assert_eq!(*events.borrow(), vec![(TouchPhase::Started, 10.0), (TouchPhase::Moved, 30.0), (TouchPhase::Moved, 250.0), (TouchPhase::Ended, 300.0)]);
    }

    #[gpui::test]
    fn cancellation_releases_control_and_outside_pan_is_not_claimed(cx: &mut TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({ let events = events.clone(); move |_, _| Control(events) });
        send(window, cx, TouchPhase::Started, 10.0);
        send(window, cx, TouchPhase::Cancelled, 30.0);
        send(window, cx, TouchPhase::Started, 200.0);
        send(window, cx, TouchPhase::Moved, 50.0);
        send(window, cx, TouchPhase::Cancelled, 50.0);
        send(window, cx, TouchPhase::Started, 20.0);
        send(window, cx, TouchPhase::Ended, 20.0);
        assert_eq!(*events.borrow(), vec![(TouchPhase::Started, 10.0), (TouchPhase::Cancelled, 30.0), (TouchPhase::Started, 20.0), (TouchPhase::Ended, 20.0)]);
    }

    #[gpui::test]
    fn parent_scrolling_yields_only_over_the_control(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| Control(Default::default()));
        window.update(cx, |_, window, _| {
            assert!(super::super::bars::over_wheel_control(window, point(px(10.0), px(10.0))));
            assert!(!super::super::bars::over_wheel_control(window, point(px(200.0), px(10.0))));
        }).unwrap();
    }
}
