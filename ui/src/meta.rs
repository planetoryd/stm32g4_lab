use iced::Element;
use plotters::style;
use plotters_iced::{Chart, ChartWidget};
use ringbuf::{
    traits::{Consumer, Observer},
    HeapRb,
};

use crate::Msg;

/// stats about the itself

pub struct MetaChart {
    reports: HeapRb<ReportStat>,
}

impl Default for MetaChart {
    fn default() -> Self {
        Self {
            reports: HeapRb::new(100),
        }
    }
}

pub struct ReportStat {
    hall_bytes_len: usize,
}

impl MetaChart {
    pub fn view(&self) -> Element<Msg> {
        ChartWidget::new(self).into()
    }
}

impl Chart<Msg> for MetaChart {
    type State = ();
    fn build_chart<DB: plotters::prelude::DrawingBackend>(
        &self,
        state: &Self::State,
        mut c: plotters::prelude::ChartBuilder<DB>,
    ) {
        use plotters::prelude::*;
        let ymax = self.reports.iter().max_by_key(|x| x.hall_bytes_len);
        let mut c = c
            .x_label_area_size(20)
            .y_label_area_size(20)
            .margin(10)
            .build_cartesian_2d(
                0..self.reports.capacity().into(),
                0..ymax.map_or(16, |k| k.hall_bytes_len),
            )
            .unwrap();
        c.configure_mesh()
            .bold_line_style(style::colors::BLUE.mix(0.2))
            .light_line_style(plotters::style::colors::BLUE.mix(0.1))
            .draw()
            .unwrap();
        c.draw_series(
            AreaSeries::new(
                self.reports
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x, y.hall_bytes_len)),
                0,
                style::colors::BLUE.mix(0.4),
            )
            .border_style(ShapeStyle::from(style::colors::BLUE).stroke_width(2)),
        )
        .unwrap();
    }
}
