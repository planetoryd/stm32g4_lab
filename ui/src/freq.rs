use std::collections::BTreeMap;

use iced::{widget::column, Element};
use plotters::{
    prelude::{BindKeyPoints, IntoLogRange},
    series::AreaSeries,
    style::{self, Color},
};
use plotters_iced::{Chart, ChartWidget};
use spectrum_analyzer::Frequency;

use crate::Msg;

#[derive(Default)]
pub struct FreqChart {
    pub freqs: BTreeMap<Frequency, Frequency>,
}

impl FreqChart {
    pub fn view(&self) -> Element<Msg> {
        let chart = ChartWidget::new(self);

        column!(chart).into()
    }
}

impl Chart<Msg> for FreqChart {
    type State = ();
    fn build_chart<DB: plotters::prelude::DrawingBackend>(
        &self,
        state: &Self::State,
        mut c: plotters::prelude::ChartBuilder<DB>,
    ) {
        if self.freqs.len() < 2 {
            return;
        }
        let min = self.freqs.first_key_value().unwrap();
        let max = self.freqs.last_key_value().unwrap();
        let points_x = self.freqs.iter().map(|x| x.0.val());
        let mut cx = c
            .x_label_area_size(20)
            .y_label_area_size(20)
            .margin(10)
            .build_cartesian_2d(
                (min.0.val()..max.0.val()),
                0f32..50f32,
            )
            .unwrap();
        cx.configure_mesh().draw().unwrap();
        cx.draw_series(AreaSeries::new(
            self.freqs.iter().map(|(x, y)| (x.val(), y.val())),
            0.,
            style::colors::BLUE.mix(0.5).filled(),
        ))
        .unwrap();
    }
}
