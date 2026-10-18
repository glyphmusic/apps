//! The phone screen as GPUI's single display.

use anyhow::Result;
use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub(crate) struct AndroidDisplay {
    pub bounds: Bounds<Pixels>,
    pub refresh_hz: f32,
}

impl PlatformDisplay for AndroidDisplay {
    fn id(&self) -> DisplayId {
        DisplayId::new(1)
    }

    fn uuid(&self) -> Result<Uuid> {
        Ok(Uuid::new_v5(&Uuid::NAMESPACE_OID, b"android-main-display"))
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    fn refresh_interval(&self) -> Option<Duration> {
        gpui::refresh_interval_from_hz(self.refresh_hz as f64)
    }
}
