use crate::*;
use plotters::prelude::*;

pub struct DACChart {
    dac_vals: HeapRb<u16>,
    adc_vals: HeapRb<u16>,
    /// photo coupler
    pcoupler: HeapRb<bool>,
}

impl Chart<Msg> for DACChart {
    type State = ();
    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, builder: ChartBuilder<DB>) {}
    fn draw_chart<DB: DrawingBackend>(
        &self,
        state: &Self::State,
        root: DrawingArea<DB, plotters::coord::Shift>,
    ) {
        let mut cb = ChartBuilder::on(&root);
        let mut cx = cb
            .x_label_area_size(20)
            .y_label_area_size(30)
            .margin(10)
            .build_cartesian_2d(0usize..self.dac_vals.capacity().get(), 0..4096u32)
            .unwrap();
        cx.configure_mesh().draw().unwrap();
        cx.draw_series(LineSeries::new(
            self.dac_vals.iter().enumerate().map(|(x, y)| (x, (*y) as u32)),
            style::colors::BLUE.mix(0.5).filled(),
        )).unwrap();
        
    }
}

impl Default for DACChart {
    fn default() -> Self {
        let num = 100;
        Self {
            dac_vals: HeapRb::new(num),
            adc_vals: HeapRb::new(num),
            pcoupler: HeapRb::new(num),
        }
    }
}

impl DACChart {
    pub fn view(&self) -> Element<'_, Msg> {
        ChartWidget::new(self).into()
    }
}
