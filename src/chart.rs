use chrono::NaiveDate;
use iced::{Element, Length};
use plotters::prelude::*;
use plotters_iced::{Chart, ChartWidget};

pub struct LocChart {
    points: Vec<(NaiveDate, u64)>,
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
}

impl Chart<crate::app::Message> for LocChart {
    type State = ();

    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &Self::State,
        mut builder: ChartBuilder<'_, '_, DB>,
    ) {
        if self.points.is_empty() {
            return;
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

        let mut chart = match builder
            .margin(20)
            .x_label_area_size(40)
            .y_label_area_size(70)
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
