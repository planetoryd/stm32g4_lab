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
    style::{self, Color, IntoTextStyle, TextStyle, BLUE},
};
use plotters_iced::{Chart, ChartBuilder, ChartWidget};
use spectrum_analyzer::Frequency;

use crate::Msg;

#[derive(Default)]
pub struct FreqChart {
    pub freqs: BTreeMap<Frequency, Frequency>,
    pub cols: Vec<Col>,
    pub top: Vec<Row>,
}

impl FreqChart {
    pub fn view(&self) -> Element<Msg> {
        let chart = ChartWidget::new(self);
        let table = iced_table::table(Id::unique(), Id::unique(), &self.cols, &self.top, |s| {
            Msg::Null
        });
        row!(chart, table)
            .align_items(iced::Alignment::Center)
            .into()
    }
}

impl Chart<Msg> for FreqChart {
    type State = ();
    fn draw_chart<DB: plotters::prelude::DrawingBackend>(
        &self,
        state: &Self::State,
        root: plotters::prelude::DrawingArea<DB, plotters::coord::Shift>,
    ) {
        if self.freqs.len() < 2 {
            return;
        }
        use plotters::prelude::*;
        let mut c = ChartBuilder::on(&root);

        let min = self.freqs.first_key_value().unwrap();
        let max = self.freqs.last_key_value().unwrap();
        // let points_x = self.freqs.iter().map(|x| x.0);
        let mut cx = c
            .x_label_area_size(20)
            .y_label_area_size(40)
            .margin(10)
            .build_cartesian_2d(min.0.val()..max.0.val(), 0f32..250f32)
            .unwrap();
        cx.configure_mesh().draw().unwrap();
        cx.draw_series(AreaSeries::new(
            self.freqs.iter().map(|(x, y)| (x.val(), y.val())),
            0.,
            style::colors::BLUE.mix(0.5).filled(),
        ))
        .unwrap();

        let text_ = plotters::style::BLUE;
        let text = text_.into_text_style(&root);
        for (ix, row) in self.top.iter().enumerate().take(3) {
            let mut pos = cx.backend_coord(&(row.freq.val(), row.val.val()));
            root.draw(&Cross::new(pos, 5, plotters::style::BLUE.mix(0.8)))
                .unwrap();
            pos.1 = 20 + ix as i32 * 20;
            root.draw_text(&row.freq.val().to_string(), &text, pos)
                .unwrap();
        }
    }
    fn build_chart<DB: plotters::prelude::DrawingBackend>(
        &self,
        _: &Self::State,
        _: plotters::prelude::ChartBuilder<DB>,
    ) {
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
    pub width: f32,
}

#[derive(Clone)]
pub struct Row {
    pub freq: Frequency,
    pub val: Frequency,
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

        container(text(con))
            .width(Length::Fill)
            .height(Length::Shrink)
            .center_x()
            .center_y()
            .into()
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
