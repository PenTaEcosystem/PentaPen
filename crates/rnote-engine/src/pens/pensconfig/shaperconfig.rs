// Imports
use rand::{RngExt, SeedableRng};
use rnote_compose::Constraints;
use rnote_compose::Style;
use rnote_compose::builders::ShapeBuilderType;
use rnote_compose::constraints::ConstraintRatio;
use rnote_compose::style::rough::RoughOptions;
use rnote_compose::style::smooth::SmoothOptions;
use serde::{Deserialize, Serialize};

#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, num_derive::FromPrimitive, num_derive::ToPrimitive,
)]
#[serde(rename = "shaper_style")]
pub enum ShaperStyle {
    #[serde(rename = "smooth")]
    Smooth = 0,
    #[serde(rename = "rough")]
    Rough,
}

impl Default for ShaperStyle {
    fn default() -> Self {
        Self::Smooth
    }
}

impl TryFrom<u32> for ShaperStyle {
    type Error = anyhow::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        num_traits::FromPrimitive::from_u32(value).ok_or_else(|| {
            anyhow::anyhow!("ShaperStyle try_from::<u32>() for value {} failed", value)
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename = "shaper_config")]
pub struct ShaperConfig {
    #[serde(rename = "builder_type")]
    pub builder_type: ShapeBuilderType,
    #[serde(rename = "style")]
    pub style: ShaperStyle,
    #[serde(rename = "smooth_options")]
    pub smooth_options: SmoothOptions,
    #[serde(rename = "rough_options")]
    pub rough_options: RoughOptions,
    #[serde(rename = "highlight_mode")]
    pub highlight_mode: bool,
    #[serde(
        rename = "highlight_opacity",
        with = "rnote_compose::serialize::f64_dp3"
    )]
    pub highlight_opacity: f64,
    #[serde(rename = "constraints")]
    pub constraints: Constraints,
    #[serde(default = "default_math_center_label", rename = "math_center_label")]
    pub math_center_label: String,
    #[serde(default = "default_true", rename = "math_auto_intersections")]
    pub math_auto_intersections: bool,
    #[serde(default = "default_true", rename = "math_show_centers")]
    pub math_show_centers: bool,
}

fn default_math_center_label() -> String {
    String::from("O")
}

fn default_true() -> bool {
    true
}

impl Default for ShaperConfig {
    fn default() -> Self {
        let mut constraints = Constraints::default();
        constraints.ratios.insert(ConstraintRatio::OneToOne);
        constraints.ratios.insert(ConstraintRatio::Horizontal);
        constraints.ratios.insert(ConstraintRatio::Vertical);

        Self {
            builder_type: ShapeBuilderType::default(),
            style: ShaperStyle::default(),
            smooth_options: SmoothOptions::default(),
            rough_options: RoughOptions::default(),
            highlight_mode: false,
            highlight_opacity: 0.45,
            constraints,
            math_center_label: String::from("O"),
            math_auto_intersections: true,
            math_show_centers: true,
        }
    }
}

impl ShaperConfig {
    pub const STROKE_WIDTH_MIN: f64 = 0.1;
    pub const STROKE_WIDTH_MAX: f64 = 500.0;

    /// A new seed for new shapes
    pub(crate) fn new_style_seeds(&mut self) {
        let seed = Some(rand_pcg::Pcg64::from_rng(&mut rand::rng()).random());
        self.rough_options.seed = seed;
    }

    pub(crate) fn gen_style_for_current_options(&self) -> Style {
        match &self.style {
            ShaperStyle::Smooth => {
                let mut options = self.smooth_options.clone();
                if self.highlight_mode {
                    if let Some(ref mut color) = options.stroke_color {
                        color.a = self.highlight_opacity;
                    }
                    if let Some(ref mut color) = options.fill_color
                        && color.a > 0.0
                    {
                        color.a = self.highlight_opacity;
                    }
                }
                Style::Smooth(options)
            }
            ShaperStyle::Rough => {
                let mut options = self.rough_options.clone();
                if self.highlight_mode {
                    if let Some(ref mut color) = options.stroke_color {
                        color.a = self.highlight_opacity;
                    }
                    if let Some(ref mut color) = options.fill_color
                        && color.a > 0.0
                    {
                        color.a = self.highlight_opacity;
                    }
                }
                Style::Rough(options)
            }
        }
    }

    /// Advance point label to next letter if it's a single ASCII uppercase letter (A -> B -> C...)
    pub fn advance_point_label(&mut self) {
        if self.math_center_label.len() == 1 {
            let c = self.math_center_label.chars().next().unwrap();
            if c.is_ascii_uppercase() {
                if c == 'Z' {
                    self.math_center_label = "A".to_string();
                } else {
                    let next = (c as u8 + 1) as char;
                    self.math_center_label = next.to_string();
                }
            }
        }
    }
}
