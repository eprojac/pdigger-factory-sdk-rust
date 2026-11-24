//! The window: [`run`] opens a [`ViewerApp`] with eframe.
//!
//! Excluded from coverage: `eframe::run_native` needs a display and a GL context, which the
//! test container does not have (see TESTING.md). Everything it shows is [`ViewerApp::ui`],
//! which the headless tests run.

use crate::ViewerApp;
use crate::craft::{Accent, Brightness};

struct Window(ViewerApp);

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

/// Opens `app` in a window titled `title` (blocks until the window closes).
pub fn run(title: &str, app: ViewerApp) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 760.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title(title),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(|cc| {
            crate::craft::theme::apply(&cc.egui_ctx, Brightness::MediumDark, Accent::Copper);
            Ok(Box::new(Window(app)))
        }),
    )
}
