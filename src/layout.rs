/// Fit the entire video into the available area without cropping or letterboxing.
pub fn fit_video(bounds: [f32; 2], aspect: f32) -> [f32; 2] {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let width = bounds[0].max(1.0).min(bounds[1].max(1.0) * aspect);
    [width, width / aspect]
}

pub fn preview_dimensions(aspect: f32) -> [usize; 2] {
    fit_video([960.0, 720.0], aspect).map(|size| size.round().max(1.0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn landscape_portrait_and_ultrawide_fit_without_cropping() {
        for aspect in [16.0 / 9.0, 9.0 / 16.0, 1.0, 32.0 / 9.0] {
            for bounds in [[800.0, 600.0], [360.0, 220.0], [1200.0, 700.0]] {
                let [w, h] = fit_video(bounds, aspect);
                assert!(w <= bounds[0] + 0.01 && h <= bounds[1] + 0.01);
                assert!((w / h - aspect).abs() < 0.001);
            }
            let [w, h] = preview_dimensions(aspect);
            assert!(w <= 960 && h <= 720);
            assert!((w as f32 / h as f32 - aspect).abs() < 0.01);
        }
    }
}
