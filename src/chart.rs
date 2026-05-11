use chrono::NaiveDate;
use iced::event::Status;
use iced::widget::canvas::Event;
use iced::{mouse, Element, Length, Rectangle};
use plotters::coord::Shift;
use plotters::prelude::*;
use plotters_iced::{Chart, ChartWidget};

const MARGIN: f32 = 20.0;
const X_LABEL_AREA: f32 = 40.0;
const Y_LABEL_AREA: f32 = 70.0;
const SNAP_THRESHOLD_PX: f32 = 30.0;
const BALL_RADIUS: i32 = 5;

pub struct LocChart {
    points: Vec<(NaiveDate, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoverInfo {
    index: usize,
    date: NaiveDate,
    value: u64,
    pixel_x: i32,
    pixel_y: i32,
}

impl LocChart {
    pub fn new(points: Vec<(NaiveDate, u64)>) -> Self {
        Self { points }
    }

    pub fn view(&self) -> Element<'_, crate::app::Message> {
        ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn axis_ranges(&self) -> Option<(NaiveDate, NaiveDate, u64)> {
        if self.points.is_empty() {
            return None;
        }
        let x_min = self.points.first().unwrap().0;
        let x_max = self.points.last().unwrap().0;
        let y_max = self
            .points
            .iter()
            .map(|(_, y)| *y)
            .max()
            .unwrap_or(1)
            .max(1);
        let y_top = y_max + (y_max / 10).max(1);
        Some((x_min, x_max, y_top))
    }

    fn plot_rect(width: f32, height: f32) -> (f32, f32, f32, f32) {
        let left = Y_LABEL_AREA + MARGIN;
        let right = width - MARGIN;
        let top = MARGIN;
        let bottom = height - X_LABEL_AREA - MARGIN;
        (left, top, right, bottom)
    }

    fn data_to_pixel(
        &self,
        date: NaiveDate,
        value: u64,
        width: f32,
        height: f32,
    ) -> Option<(f32, f32)> {
        let (left, top, right, bottom) = Self::plot_rect(width, height);
        if right <= left || bottom <= top {
            return None;
        }
        let (x_min, x_max, y_top) = self.axis_ranges()?;
        let x_days = (x_max - x_min).num_days() as f32;
        let date_days = (date - x_min).num_days() as f32;
        let px = if x_days <= 0.0 {
            (left + right) / 2.0
        } else {
            left + (date_days / x_days) * (right - left)
        };
        let py = top + (1.0 - (value as f32 / y_top as f32)) * (bottom - top);
        Some((px, py))
    }

    fn compute_hover(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<HoverInfo> {
        let pos = cursor.position()?;
        let rel_x = pos.x - bounds.x;
        let rel_y = pos.y - bounds.y;
        let (left, top, right, bottom) = Self::plot_rect(bounds.width, bounds.height);
        if rel_x < left || rel_x > right || rel_y < top || rel_y > bottom {
            return None;
        }
        if self.points.is_empty() {
            return None;
        }
        let (x_min, x_max, _y_top) = self.axis_ranges()?;
        let plot_w = right - left;
        if plot_w <= 0.0 {
            return None;
        }
        let ratio = ((rel_x - left) / plot_w).clamp(0.0, 1.0);
        let x_days = (x_max - x_min).num_days() as f32;
        let target_days = ratio * x_days;

        let mut best_idx = 0usize;
        let mut best_diff = f32::MAX;
        for (i, (date, _)) in self.points.iter().enumerate() {
            let d = ((*date - x_min).num_days() as f32 - target_days).abs();
            if d < best_diff {
                best_diff = d;
                best_idx = i;
            }
        }

        let (date, value) = self.points[best_idx];
        let (px, py) = self.data_to_pixel(date, value, bounds.width, bounds.height)?;

        if (rel_y - py).abs() > SNAP_THRESHOLD_PX {
            return None;
        }

        Some(HoverInfo {
            index: best_idx,
            date,
            value,
            pixel_x: px.round() as i32,
            pixel_y: py.round() as i32,
        })
    }
}

impl Chart<crate::app::Message> for LocChart {
    type State = Option<HoverInfo>;

    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &Self::State,
        _builder: ChartBuilder<'_, '_, DB>,
    ) {
    }

    fn draw_chart<DB: DrawingBackend>(
        &self,
        state: &Self::State,
        root: DrawingArea<DB, Shift>,
    ) {
        if self.points.is_empty() {
            return;
        }
        let Some((x_min, x_max, y_top)) = self.axis_ranges() else {
            return;
        };

        let mut chart = match ChartBuilder::on(&root)
            .margin(MARGIN as u32)
            .x_label_area_size(X_LABEL_AREA as u32)
            .y_label_area_size(Y_LABEL_AREA as u32)
            .build_cartesian_2d(x_min..x_max, 0u64..y_top)
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let _ = chart
            .configure_mesh()
            .x_labels(8)
            .y_labels(6)
            .light_line_style(RGBColor(230, 230, 230))
            .bold_line_style(RGBColor(200, 200, 200))
            .x_label_formatter(&|d| d.format("%Y-%m-%d").to_string())
            .y_label_formatter(&|v| format_count(*v))
            .draw();

        let _ = chart.draw_series(LineSeries::new(
            self.points.iter().copied(),
            ShapeStyle::from(RGBColor(31, 119, 180)).stroke_width(2),
        ));

        let Some(hover) = state else { return };
        if hover.index >= self.points.len()
            || self.points[hover.index] != (hover.date, hover.value)
        {
            return;
        }

        let _ = chart.plotting_area().draw(&Circle::new(
            (hover.date, hover.value),
            BALL_RADIUS,
            ShapeStyle::from(RGBColor(31, 119, 180)).filled(),
        ));
        let _ = chart.plotting_area().draw(&Circle::new(
            (hover.date, hover.value),
            BALL_RADIUS + 1,
            ShapeStyle::from(RGBColor(255, 255, 255)).stroke_width(1),
        ));

        let date_str = hover.date.format("%Y-%m-%d").to_string();
        let value_str = format!("{} LOC", format_count(hover.value));
        let max_chars = date_str.len().max(value_str.len()) as i32;
        let tooltip_w = max_chars * 8 + 16;
        let tooltip_h = 40i32;
        let pad = 8i32;
        let line_h = 18i32;

        let (root_w, root_h) = root.dim_in_pixel();
        let (plot_left, _plot_top, plot_right, _plot_bottom) =
            Self::plot_rect(root_w as f32, root_h as f32);
        let mid_x = (plot_left + plot_right) / 2.0;

        let mut tx = if (hover.pixel_x as f32) < mid_x {
            hover.pixel_x + 14
        } else {
            hover.pixel_x - 14 - tooltip_w
        };
        let min_tx = plot_left as i32;
        let max_tx = (plot_right as i32) - tooltip_w;
        if max_tx >= min_tx {
            tx = tx.clamp(min_tx, max_tx);
        }

        let mut ty = hover.pixel_y - tooltip_h / 2;
        let min_ty = MARGIN as i32;
        let max_ty = (root_h as i32) - (X_LABEL_AREA as i32) - (MARGIN as i32) - tooltip_h;
        if max_ty >= min_ty {
            ty = ty.clamp(min_ty, max_ty);
        }

        let _ = root.draw(&plotters::element::Rectangle::new(
            [(tx, ty), (tx + tooltip_w, ty + tooltip_h)],
            ShapeStyle::from(RGBColor(255, 255, 255)).filled(),
        ));
        let _ = root.draw(&plotters::element::Rectangle::new(
            [(tx, ty), (tx + tooltip_w, ty + tooltip_h)],
            ShapeStyle::from(RGBColor(180, 180, 180)).stroke_width(1),
        ));

        let text_color = RGBColor(30, 30, 30);
        let text_style = ("sans-serif", 14u32).into_font().color(&text_color);
        let _ = root.draw(&plotters::element::Text::new(
            date_str,
            (tx + pad, ty + pad),
            text_style.clone(),
        ));
        let _ = root.draw(&plotters::element::Text::new(
            value_str,
            (tx + pad, ty + pad + line_h),
            text_style,
        ));
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<crate::app::Message>) {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let new_state = self.compute_hover(bounds, cursor);
                if new_state != *state {
                    *state = new_state;
                    (Status::Captured, None)
                } else {
                    (Status::Ignored, None)
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                if state.is_some() {
                    *state = None;
                    (Status::Captured, None)
                } else {
                    (Status::Ignored, None)
                }
            }
            _ => (Status::Ignored, None),
        }
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.is_some() {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

fn format_count(v: u64) -> String {
    if v >= 1_000_000 {
        format!("{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.1}k", v as f64 / 1_000.0)
    } else {
        v.to_string()
    }
}
