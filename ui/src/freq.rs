use std::collections::BTreeMap;

use iced::{
    widget::{column, container, row, scrollable::Id, text},
    Element, Length, Renderer, Theme,
};
use iced_aw::{grid, grid_row};
use iced_table::table;
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
    pub freqs: BTreeMap<u32, f32>,
    pub cols: Vec<Col>,
    pub rows: Vec<Row>,
}

impl FreqChart {
    pub fn view(&self) -> Element<Msg> {
        let chart = ChartWidget::new(self);
        let table = iced_table::table(Id::unique(), Id::unique(), &self.cols, &self.rows, |s| {
            Msg::Null
        });
        row!(chart, table)
            .align_items(iced::Alignment::Center)
            .into()
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
        let points_x = self.freqs.iter().map(|x| x.0);
        let mut cx = c
            .x_label_area_size(20)
            .y_label_area_size(40)
            .margin(10)
            .build_cartesian_2d((*min.0..*max.0), 0f32..50f32)
            .unwrap();
        cx.configure_mesh().draw().unwrap();
        cx.draw_series(AreaSeries::new(
            self.freqs.iter().map(|(x, y)| (*x, *y)),
            0.,
            style::colors::BLUE.mix(0.5).filled(),
        ))
        .unwrap();
    }
}

#[derive(Clone)]
pub enum Colkind {
    Freq,
    Val,
}

#[derive(Clone)]
pub struct Col {
    pub kind: Colkind,
    pub width: f32
}

#[derive(Clone)]
pub struct Row {
    pub freq: f32,
    pub val: f32
}

impl<'a> table::Column<'a, Msg, Theme, Renderer> for Col {
    type Row = Row;
    fn header(
        &'a self,
        col_index: usize,
    ) -> iced::advanced::graphics::core::Element<'a, Msg, Theme, Renderer> {
        let con = match self.kind {
            Colkind::Freq => "Freq",
            Colkind::Val => "Val",
        };

        container(text(con)).width(Length::Fill).height(Length::Shrink).center_x().center_y().into()
    }
    fn cell(
        &'a self,
        col_index: usize,
        row_index: usize,
        row: &'a Self::Row,
    ) -> iced::advanced::graphics::core::Element<'a, Msg, Theme, Renderer> {
        let con: Element<_> = match self.kind {
            Colkind::Freq => text(row.freq).into(),
            Colkind::Val => text(row.val).into(),
        };

        container(con)
            .width(Length::Fill)
            .height(Length::Shrink)
            .center_y()
            .into()
    }
    fn footer(
        &'a self,
        _col_index: usize,
        _rows: &'a [Self::Row],
    ) -> Option<iced::advanced::graphics::core::Element<'a, Msg, Theme, Renderer>> {
        None
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn resize_offset(&self) -> Option<f32> {
        None
    }
}
