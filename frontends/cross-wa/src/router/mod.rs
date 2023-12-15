pub mod bindings;
mod constants;
pub mod mouse;
mod route;

use raw_window_handle::{HasRawWindowHandle, HasRawDisplayHandle};
use crate::renderer::{padding_top_from_config, padding_bottom_from_config};
use crate::event::{RioEvent, UpdateOpcode};
use crate::ime::Ime;
use crate::scheduler::{Scheduler, TimerId, Topic};
use rio_backend::event::EventPayload;
use rio_backend::superloop::Superloop;
use route::Route;
use std::collections::HashMap;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;
use sugarloaf::font::loader;

use wa::*;

struct Router {
    config: Rc<rio_backend::config::Config>,
    routes: HashMap<u8, Route>,
    current: u8,
    superloop: Superloop,
    scheduler: Scheduler,
    font_database: loader::Database,
}

impl EventHandler for Router {
    fn init(
        &mut self,
        id: u8,
        raw_window_handle: raw_window_handle::RawWindowHandle,
        raw_display_handle: raw_window_handle::RawDisplayHandle,
        width: i32,
        height: i32,
        scale_factor: f32,
    ) {
        let initial_route = Route::new(
            id.into(),
            raw_window_handle,
            raw_display_handle,
            self.config.clone(),
            self.superloop.clone(),
            &self.font_database,
            width,
            height,
            scale_factor,
        )
        .unwrap();
        self.routes.insert(id, initial_route);
    }
    #[inline]
    fn process(&mut self) -> EventHandlerAction {
        let mut next = EventHandlerAction::Noop;

        // TODO:
        // match self.scheduler.update() {
        //     Some(instant) => { return next },
        //     None => {},
        // };

        match self.superloop.event() {
            RioEvent::Render | RioEvent::Wakeup => {
                return EventHandlerAction::Render;
            }
            RioEvent::PowerOn => {
                next = EventHandlerAction::Init;
            }
            RioEvent::Paste => {
                if let Some(value) = window::clipboard_get() {
                    if let Some(current) = self.routes.get_mut(&self.current) {
                        current.paste(&value, true);
                        next = EventHandlerAction::Render;
                    }
                }
            }
            RioEvent::Copy(data) => {
                window::clipboard_set(&data);
            }
            RioEvent::UpdateConfig => {
                let (config, _config_error) =
                    match rio_backend::config::Config::try_load() {
                        Ok(config) => (config, None),
                        Err(error) => {
                            (rio_backend::config::Config::default(), Some(error))
                        }
                    };

                self.config = config.into();
                // for (_id, route) in self.router.routes.iter_mut() {
                // route.update_config(
                //     &self.config,
                //     &self.router.font_database,
                // );

                // self.window
                //     .screen
                //     .update_config(config, self.window.winit_window.theme(), db);

                if let Some(current) = self.routes.get_mut(&self.current) {
                    current.update_config(&self.config);
                }

                // if let Some(error) = &config_error {
                //     route.report_error(&error.to_owned().into());
                // } else {
                //     route.clear_errors();
                // }
                // }
                next = EventHandlerAction::Render;
            }
            RioEvent::Title(title) => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    window::set_window_title(title);
                }
            }
            RioEvent::CreateNativeTab(_) => {}
            RioEvent::MouseCursorDirty => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    current.mouse.accumulated_scroll =
                        mouse::AccumulatedScroll::default();
                }
            }
            RioEvent::Scroll(scroll) => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    let mut terminal = current.ctx.current().terminal.lock();
                    terminal.scroll_display(scroll);
                    drop(terminal);
                }
            }
            RioEvent::ClipboardLoad(clipboard_type, format) => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    // if route.window.is_focused {
                    let text = format(current.clipboard_get(clipboard_type).as_str());
                    current
                        .ctx
                        .current_mut()
                        .messenger
                        .send_bytes(text.into_bytes());
                    // }
                }
            }
            RioEvent::ClipboardStore(clipboard_type, content) => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    // if current.is_focused {
                    current.clipboard_store(clipboard_type, content);
                    // }
                }
            }
            RioEvent::PtyWrite(text) => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    current
                        .ctx
                        .current_mut()
                        .messenger
                        .send_bytes(text.into_bytes());
                }
            }
            RioEvent::UpdateFontSize(action) => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    let should_update = match action {
                        0 => current.sugarloaf.layout.reset_font_size(),
                        2 => current.sugarloaf.layout.increase_font_size(),
                        1 => current.sugarloaf.layout.decrease_font_size(),
                        _ => false,
                    };

                    if !should_update {
                        return EventHandlerAction::Noop;
                    }

                    // This is a hacky solution, sugarloaf compute bounds in runtime
                    // so basically it updates with the new font-size, then compute the bounds
                    // and then updates again with correct bounds
                    current.sugarloaf.layout.update();
                    current.sugarloaf.calculate_bounds();
                    current.sugarloaf.layout.update();

                    current.resize_all_contexts();
                }

                next = EventHandlerAction::Render;
            }
            RioEvent::RequestUpdate(opcode) => {
                next = EventHandlerAction::Update(opcode);
            }
            // RioEvent::ScheduleDraw(millis) => {
            //     let timer_id = TimerId::new(Topic::Render, 0);
            //     let event = EventPayload::new(RioEvent::Render, self.current);

            //     if !self.scheduler.scheduled(timer_id) {
            //         self.scheduler.schedule(
            //             event,
            //             Duration::from_millis(millis),
            //             false,
            //             timer_id,
            //         );
            //     }
            // }
            RioEvent::Noop | _ => {}
        };

        next
    }

    // Update needs to be async with a wait
    fn update(&mut self, opcode: u8) {
        match opcode.into() {
            UpdateOpcode::UpdateGraphicLibrary => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    let mut terminal = current.ctx.current().terminal.lock();
                    let graphics = terminal.graphics_take_queues();
                    if let Some(graphic_queues) = graphics {
                        let renderer = &mut current.sugarloaf;
                        for graphic_data in graphic_queues.pending {
                            renderer.graphics.add(graphic_data);
                        }

                        for graphic_data in graphic_queues.remove_queue {
                            renderer.graphics.remove(&graphic_data);
                        }
                    }
                }
            }
            UpdateOpcode::ForceRefresh => {
                if let Some(current) = self.routes.get_mut(&self.current) {
                    if let Some(_err) = current
                        .sugarloaf
                        .update_font(self.config.fonts.to_owned(), None)
                    {
                        // self.context_manager
                        // .report_error_fonts_not_found(err.fonts_not_found);
                        return;
                    }

                    let padding_y_bottom = padding_bottom_from_config(&self.config);
                    let padding_y_top = padding_top_from_config(&self.config);

                    current.sugarloaf.layout.recalculate(
                        self.config.fonts.size,
                        self.config.line_height,
                        self.config.padding_x,
                        padding_y_top,
                        padding_y_bottom,
                    );

                    current.sugarloaf.layout.update();

                    current.mouse.set_multiplier_and_divider(
                        self.config.scroll.multiplier,
                        self.config.scroll.divider,
                    );

                    current.resize_all_contexts();

                    let mut bg_color = current.state.named_colors.background.1;

                    if self.config.window.background_opacity < 1. {
                        bg_color.a = self.config.window.background_opacity as f64;
                    }

                    current.sugarloaf.set_background_color(bg_color);
                    if let Some(image) = &self.config.window.background_image {
                        current.sugarloaf.set_background_image(&image);
                    }

                    current.sugarloaf.calculate_bounds();
                    current.sugarloaf.render();
                }
            }
        }
    }

    #[inline]
    fn draw(&mut self) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            current.render();
        }
    }

    fn key_down_event(
        &mut self,
        keycode: KeyCode,
        mods: ModifiersState,
        repeat: bool,
        character: Option<smol_str::SmolStr>,
    ) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            if keycode == KeyCode::LeftSuper || keycode == KeyCode::RightSuper {
                if current.search_nearest_hyperlink_from_pos() {
                    window::set_mouse_cursor(wa::CursorIcon::Pointer);
                    self.superloop.send_event(RioEvent::Render, self.current);
                    return;
                }
            }

            current.process_key_event(keycode, mods, true, repeat, character);
        }
    }
    fn key_up_event(&mut self, keycode: KeyCode, mods: ModifiersState) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            current.process_key_event(keycode, mods, false, false, None);
            current.render();
        }
    }
    fn mouse_motion_event(&mut self, x: f32, y: f32) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            if self.config.hide_cursor_when_typing {
                window::show_mouse(true);
            }

            if let Some(cursor) = current.process_motion_event(x, y) {
                window::set_mouse_cursor(cursor);
            }

            current.render();
        }
    }
    fn touch_event(&mut self, phase: TouchPhase, _id: u64, _x: f32, _y: f32) {
        if phase == TouchPhase::Started {
            if let Some(current) = self.routes.get_mut(&self.current) {
                current.mouse.accumulated_scroll = Default::default();
            }
        }
    }
    fn mouse_wheel_event(&mut self, mut x: f32, mut y: f32) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            // if route.path != RoutePath::Terminal {
            //     return;
            // }

            if self.config.hide_cursor_when_typing {
                window::show_mouse(true);
            }

            // match delta {
            //     MouseScrollDelta::LineDelta(columns, lines) => {
            //         let new_scroll_px_x = columns
            //             * route.window.screen.sugarloaf.layout.font_size;
            //         let new_scroll_px_y = lines
            //             * route.window.screen.sugarloaf.layout.font_size;
            //         route.window.screen.scroll(
            //             new_scroll_px_x as f64,
            //             new_scroll_px_y as f64,
            //         );
            //     }

            // When the angle between (x, 0) and (x, y) is lower than ~25 degrees
            // (cosine is larger that 0.9) we consider this scrolling as horizontal.
            if x.abs() / x.hypot(y) > 0.9 {
                y = 0.;
            } else {
                x = 0.;
            }

            current.scroll(x.into(), y.into());
            current.render();
        }
    }
    fn mouse_button_down_event(&mut self, button: MouseButton, x: f32, y: f32) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            if self.config.hide_cursor_when_typing {
                window::show_mouse(true);
            }

            current.process_mouse(button, x, y, true);
        }
    }
    fn mouse_button_up_event(&mut self, button: MouseButton, x: f32, y: f32) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            if self.config.hide_cursor_when_typing {
                window::show_mouse(true);
            }

            current.process_mouse(button, x, y, false);
        }
    }
    fn resize_event(&mut self, w: i32, h: i32, scale_factor: f32, rescale: bool) {
        if let Some(current) = self.routes.get_mut(&self.current) {
            // let s = d.sugarloaf.clone().unwrap();
            // let mut s = s.lock();
            current
                .sugarloaf
                .resize(w.try_into().unwrap(), h.try_into().unwrap());
            if rescale {
                current.sugarloaf.rescale(scale_factor);
                current
                    .sugarloaf
                    .resize(w.try_into().unwrap(), h.try_into().unwrap());
                current.sugarloaf.calculate_bounds();
            } else {
                current
                    .sugarloaf
                    .resize(w.try_into().unwrap(), h.try_into().unwrap());
            }
            current.resize_all_contexts();
        }
    }

    fn quit_requested_event(&mut self) {
        // window::cancel_quit();
    }

    fn files_dropped_event(&mut self) {
        // println!("{:?}", window::dropped_file_path(0));
    }
}

#[inline]
pub async fn run(
    config: rio_backend::config::Config,
    _config_error: Option<rio_backend::config::ConfigError>,
) -> Result<(), Box<dyn Error>> {
    let mut superloop = Superloop::new();

    let config = Rc::new(config);

    let _ =
        crate::watcher::watch(rio_backend::config::config_dir_path(), superloop.clone());

    let scheduler = Scheduler::new(superloop.clone());

    let mut font_database = loader::Database::new();
    font_database.load_system_fonts();

    superloop.send_event(RioEvent::PowerOn, 0);

    let mut router = Router {
        config: config.clone(),
        current: 1,
        routes: HashMap::new(),
        superloop: superloop.clone(),
        scheduler,
        font_database: font_database.clone(),
    };

    let wa_conf = conf::Conf {
        window_title: String::from("~"),
        window_width: config.window.width,
        window_height: config.window.height,
        fullscreen: config.window.is_fullscreen(),
        transparency: config.window.background_opacity < 1.,
        blur: config.window.blur,
        hide_toolbar: !config.navigation.is_native(),
        hide_toolbar_buttons: config.window.macos_hide_toolbar_buttons,
        ..Default::default()
    };

    let app: wa::native::macos::App = wa::native::macos::App::new();
    // spawn(async {
    let window = wa::native::macos::Window::new_window(
        "aa",
        "Rio",
        wa_conf,
        || Box::new(router)
    ).await;
    let refwindow = window.unwrap();
    // println!("window from new_window {:?}", refwindow.ns_window);
    // println!("view from new_window {:?}", refwindow.ns_view);
    // });
    // app.create_window(wa_conf, |window| {

    // });
    app.run();

    Ok(())
}