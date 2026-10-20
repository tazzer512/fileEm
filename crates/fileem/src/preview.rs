//! The preview's zoom (the Java `PreviewWindow` zoom and fit), free of the UI toolkit. Images
//! scale their pixels, text its font; "fit" shows the whole image and never enlarges it.

pub const MIN_ZOOM: f64 = 0.05;
pub const MAX_ZOOM: f64 = 32.0;
pub const ZOOM_STEP: f64 = 1.25;
/// The text font's size at 100 %.
pub const TEXT_SIZE: f64 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    /// Images: the scale of their pixels (1 = 100 %). Text: the scale of the font.
    pub zoom: f64,
    /// The image follows the window's size.
    pub fitting: bool,
}

impl Default for Zoom {
    fn default() -> Self {
        Zoom { zoom: 1.0, fitting: true }
    }
}

impl Zoom {
    pub fn set(&mut self, z: f64) {
        self.fitting = false;
        self.zoom = z.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn by(&mut self, factor: f64) {
        self.set(self.zoom * factor);
    }

    /// Fit to the window: images follow it from now on ([`Zoom::follow`]), text goes back to its
    /// normal size.
    pub fn fit(&mut self, image: Option<(f64, f64)>, view: (f64, f64)) {
        if image.is_none() {
            self.set(1.0);
            return;
        }
        self.fitting = true;
        self.follow(image, view);
    }

    /// The window (or the image) changed: a fitted image takes the scale that shows all of it.
    pub fn follow(&mut self, image: Option<(f64, f64)>, view: (f64, f64)) {
        if let (true, Some((w, h))) = (self.fitting, image) {
            let (vw, vh) = (view.0.max(1.0), view.1.max(1.0));
            self.zoom = (vw / w.max(1.0)).min(vh / h.max(1.0)).min(1.0);
        }
    }

    pub fn percent(&self) -> i32 {
        (self.zoom * 100.0).round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shows_the_whole_image_and_never_enlarges() {
        let mut z = Zoom::default();
        z.fit(Some((2000.0, 1000.0)), (500.0, 400.0));
        assert!(z.fitting);
        assert_eq!(z.zoom, 0.25, "the width decides");
        z.follow(Some((2000.0, 1000.0)), (1000.0, 200.0));
        assert_eq!(z.zoom, 0.2, "the window got lower: the height decides");
        z.follow(Some((100.0, 50.0)), (1000.0, 800.0));
        assert_eq!(z.zoom, 1.0, "small images stay at 100 %");
    }

    #[test]
    fn zooming_steps_stops_fitting_and_stays_within_limits() {
        let mut z = Zoom::default();
        z.fit(Some((2000.0, 1000.0)), (500.0, 400.0));
        z.set(1.0);
        z.by(ZOOM_STEP);
        assert_eq!(z.zoom, 1.25);
        assert!(!z.fitting);
        z.follow(Some((2000.0, 1000.0)), (10.0, 10.0));
        assert_eq!(z.zoom, 1.25, "a chosen zoom ignores the window");
        z.by(1.0 / ZOOM_STEP);
        z.by(1.0 / ZOOM_STEP);
        assert!((z.zoom - 0.8).abs() < 1e-9);
        z.set(1000.0);
        assert_eq!(z.zoom, MAX_ZOOM);
        z.set(0.0);
        assert_eq!(z.zoom, MIN_ZOOM);
        assert_eq!(z.percent(), 5);
        // text: fit means the normal size
        z.fit(None, (500.0, 400.0));
        assert_eq!((z.zoom, z.fitting), (1.0, false));
    }
}
