use std::{collections::HashSet, fmt::format};

use crate::*;
use bittle::prelude::*;
use button::Appearance;
use common::{VMap, BALANCE_BYTES};
use fraction::Fraction;
use iced::{
    mouse,
    theme::{self, Container},
    Point, Size, Vector,
};
use iced_aw::{
    drop_down::Offset,
    floating_element::{self, Anchor},
    style::colors,
};
use linfa_linear::FittedLinearRegression;
use ndarray::arr2;
use plotters::{coord::ReverseCoordTranslate, element::PointCollection, prelude::*};
use widget::{canvas::Event, themer, Button};

pub struct BalanceChart {
    pub measurements: HeapRb<u16>,
    /// photo coupler
    pub pcoupler: bittle::Set<[u8; BALANCE_BYTES]>,
    /// omit outliers
    pub omit: u32,
    pub select: Option<(Point<i32>, Point<i32>)>,
    pub cur_moving: bool,
    /// nominal weight in mg to val
    pub refweight: BTreeMap<RefWeight, Fraction>,
    pub balance_rp: HeapRb<BalanceReport>,
    pub vmaps: HeapRb<VMap>,
    pub speed: HeapRb<i32>,
}

pub const RP_LEN: usize = 8192 * 2;
pub const SPEED_LEN: usize = 512;

pub static SELECTED_POINTS: RwLock<Option<HashSet<(usize, u32)>>> = RwLock::const_new(None);
pub static LINREG: RwLock<Option<FittedLinearRegression<f64>>> = RwLock::const_new(None);

#[derive(Default)]
pub struct ChartState {
    show_raw: bool,
}

impl Chart<Msg> for BalanceChart {
    type State = ChartState;

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, builder: ChartBuilder<DB>) {}
    fn draw_chart<DB: DrawingBackend>(
        &self,
        state: &Self::State,
        root: DrawingArea<DB, plotters::coord::Shift>,
    ) {
        let mut cb = ChartBuilder::on(&root);

        let mut cx = cb
            .x_label_area_size(20)
            .y_label_area_size(if state.show_raw { 80 } else { 30 })
            .margin(10)
            .build_cartesian_2d(0usize..self.pcoupler.bits_capacity() as usize, 0..5u32)
            .unwrap();
        let pc = self.pcoupler.iter_ones().map(|ix| {
            Rectangle::new(
                [(ix as usize, 0), (ix as usize + 1, 1)],
                style::colors::BLUE.mix(0.6).filled(),
            )
        });
        cx.draw_series(pc).unwrap();

        let mut vals: Vec<_> = self
            .measurements
            .iter()
            .enumerate()
            .map(|(x, y)| (x, (*y) as u32))
            .collect();
        vals.sort_by_key(|x| x.1);
        let left = self.omit as usize / 2;
        let right = self.omit as usize - left;
        let take = vals.len() - left - right;
        let mut vals_om: Vec<_> = vals.into_iter().skip(left).take(take).collect();
        vals_om.sort_by_key(|(x, y)| *x);
        let mut cb = ChartBuilder::on(&root);

        let vymin: Option<_> = vals_om.iter().min_by_key(|x| x.1).map(|x| x.1);
        let vymax = vals_om.iter().max_by_key(|x| x.1).map(|x| x.1);
        let getref = |nomimal: u32| {
            (
                self.refweight
                    .get(&RefWeight::Num(nomimal))
                    .map(|x| *x.floor().numer().unwrap() as u32),
                nomimal,
            )
        };
        let (refmin, nom_min) = getref(1);
        let (refmax, nom_max) = getref(10);
        let ymin = refmin
            .map(|x| x)
            .or(vymin)
            .map(|x| x)
            .or(Some(1000))
            .unwrap();
        let ymax = refmax
            .map(|x| x + 100)
            .or(vymax)
            .map(|x| x + 5)
            .or(Some(4095))
            .unwrap();
        let xcord = 0usize..self.measurements.capacity().get();
        let mut cx = cb
            .x_label_area_size(20)
            .y_label_area_size(30)
            .margin(10)
            .build_cartesian_2d(xcord.clone(), ymin..ymax)
            .unwrap();

        cx.draw_series(LineSeries::new(
            vals_om.clone(),
            style::colors::BLUE.mix(0.4).filled().stroke_width(6),
        ))
        .unwrap();

        let area = if let Some(rect) = self.select {
            let topleft = (min(rect.0.x, rect.1.x), min(rect.0.y, rect.1.y));
            let p = Point::new(topleft.0, topleft.1);
            let v1 = p - rect.0;
            let v2 = p - rect.1;
            let v3 = v1 + v2;
            let size: Size<f32> = Size::new(v3.x.abs() as f32, v3.y.abs() as f32);
            let pf = Point::new(p.x as f32, p.y as f32);
            let area = iced::Rectangle::new(pf, size);

            let tx = |p: Point<i32>| (p.x as i32, p.y as i32);
            let e = Rectangle::new(
                [tx(rect.0), tx(rect.1)],
                style::colors::BLUE.mix(0.3).filled(),
            );

            // let e = Circle::new(cord, 5, style::colors::BLUE.mix(0.8).stroke_width(2));
            root.use_screen_coord().draw(&e).unwrap();
            Some(area)
        } else {
            None
        };
        let mut sp = SELECTED_POINTS.blocking_write();
        let spts = sp.insert(Default::default());
        let it: Vec<_> = vals_om
            .into_iter()
            .map(|(x, y)| {
                Circle::new((x as usize, (y) as u32), 5u32, {
                    let tx = cx.as_coord_spec().translate(&(x, y));
                    if let Some(area) = area {
                        if area.contains(Point::new(tx.0 as f32, tx.1 as f32)) {
                            spts.insert((x, y));
                            style::colors::BLUE.mix(0.8).filled().stroke_width(2)
                        } else {
                            style::colors::BLUE.mix(0.8).stroke_width(2)
                        }
                    } else {
                        style::colors::BLUE.mix(0.8).stroke_width(2)
                    }
                })
            })
            .collect();
        cx.draw_series(it).unwrap();
        use linfa::prelude::*;
        use linfa::traits::Fit;
        use linfa_linear::LinearRegression;
        use ndarray::prelude::*;

        let pred = LINREG.blocking_read();
        if let Some(pred) = &*pred {
            let fmt = |k: &u32| {
                let recs = arr2(&[[*k as f64]]);
                let mut res = arr1(&[0.]);
                pred.predict_inplace(&recs, &mut res);
                if state.show_raw {
                    format!("{}", k)
                } else {
                    format!("{:.1}", res[0])
                }
            };
            // cx.configure_mesh().y_label_formatter(&fmt).draw().unwrap();
        } else {
            // cx.configure_mesh().draw().unwrap();
        }

        drop(sp);

        let sel = SELECTED_POINTS.blocking_read();
        let df = "avg=".to_owned();
        let tx = if let Some(ref inn) = *sel {
            if inn.len() > 0 {
                let avg = inn.iter().map(|x| x.1 as usize).sum::<usize>() / inn.len();
                if let Some(pred) = &*pred {
                    let recs = arr2(&[[avg as f64]]);
                    let mut res = arr1(&[0.]);
                    pred.predict_inplace(&recs, &mut res);
                    format!("avg={}={}mg", avg, res[0])
                } else {
                    format!("avg={}", avg)
                }
            } else {
                df
            }
        } else {
            df
        };

        if self.balance_rp.occupied_len() > 0 {
            let fb_max = self
                .balance_rp
                .iter()
                .max_by_key(|x| x.feedback)
                .map(|x| x.feedback as i32 + 20);
            let fb_min = self
                .balance_rp
                .iter()
                .min_by_key(|x| x.feedback)
                .map(|x| x.feedback as i32 - 20);
            let mut cb = ChartBuilder::on(&root);
            let mut cx = cb
                .x_label_area_size(20)
                .y_label_area_size(30)
                .margin(10)
                .build_cartesian_2d(
                    0..RP_LEN as i32,
                    fb_min.unwrap_or(0)..fb_max.unwrap_or(4095i32),
                )
                .unwrap();
            cx.draw_series(LineSeries::new(
                self.balance_rp
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x as i32, y.feedback as i32)),
                plotters::style::full_palette::TEAL.mix(0.8),
            ))
            .unwrap();
            let v_max = self.speed.iter().max().map(|x| *x + 1000);
            let v_min = self.speed.iter().min().map(|x| *x - 1000);
            if let (Some(vmin), Some(vmax)) = (v_min, v_max) {
                let mut c2 =
                    cx.set_secondary_coord(0..SPEED_LEN as i32, vmin as i32..(vmax as i32));
                c2.draw_secondary_series(LineSeries::new(
                    self.speed
                        .iter()
                        .enumerate()
                        .map(|(x, y)| (x as i32, *y as i32)),
                    plotters::style::full_palette::ORANGE.mix(0.8),
                ))
                .unwrap();
                c2.configure_mesh().draw().unwrap();
            } else {
                cx.configure_mesh().draw().unwrap();
            }

            let mut cb = ChartBuilder::on(&root);
            let mut cx = cb
                .x_label_area_size(20)
                .y_label_area_size(30)
                .margin(10)
                .build_cartesian_2d(0..RP_LEN as i32, 0..4095i32)
                .unwrap();
            cx.draw_series(LineSeries::new(
                self.balance_rp
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x as i32, y.photoc as i32)),
                plotters::style::full_palette::TEAL.mix(0.8),
            ))
            .unwrap();

            let mut cb = ChartBuilder::on(&root);
            let ymax = self.balance_rp.iter().max_by_key(|x| x.hall);
            let ymin = self.balance_rp.iter().min_by_key(|x| x.hall);
            let y_max = ymax.map(|x| x.hall as i32).unwrap_or(4095);
            let y_min = ymin.map(|x| x.hall as i32).unwrap_or(0);
            let mut cx = cb
                .x_label_area_size(20)
                .y_label_area_size(30)
                .margin(10)
                .build_cartesian_2d(0..RP_LEN as i32, y_min..y_max)
                .unwrap();

            cx.draw_series(LineSeries::new(
                self.balance_rp
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x as i32, y.hall as i32)),
                plotters::style::full_palette::AMBER.mix(0.8),
            ))
            .unwrap();
        }

        if self.vmaps.occupied_len() > 0 {
            let mut cb = ChartBuilder::on(&root);
            let mut cx = cb
                .x_label_area_size(20)
                .y_label_area_size(30)
                .margin(10)
                .build_cartesian_2d(0..VMAP_LEN as i32, 0..4095i32)
                .unwrap();
            cx.draw_series(LineSeries::new(
                self.vmaps
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x as i32, y.dac as i32)),
                plotters::style::full_palette::TEAL.mix(0.8),
            ))
            .unwrap();
            cx.draw_series(LineSeries::new(
                self.vmaps
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x as i32, y.fbavg as i32)),
                plotters::style::full_palette::PURPLE.mix(0.8),
            ))
            .unwrap();
        }

        root.draw_text(
            &tx,
            &style::BLACK.mix(0.9).into_text_style(&root),
            (160, 20),
        )
        .unwrap();
    }
    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.cur_moving {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::Idle
        }
    }
    fn update(
        &self,
        state: &mut Self::State,
        event: Event,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> (iced::event::Status, Option<Msg>) {
        if let Event::Mouse(mouse::Event::CursorMoved { position: pos }) = event {
            let cur = mouse::Cursor::Available(pos);
            let pos = cur.position_in(bounds);
            if let Some(c) = pos {
                if self.cur_moving {
                    return (
                        iced::event::Status::Captured,
                        Some(Msg::BaSelect(BaSelect::Move(pos.unwrap()))),
                    );
                }
            }
        }

        if let Some(c) = cursor.position_in(bounds) {
            match event {
                Event::Mouse(mouse::Event::ButtonPressed(bp)) => match bp {
                    mouse::Button::Left => match cursor {
                        mouse::Cursor::Available(_) => {
                            if !self.cur_moving {
                                return (
                                    iced::event::Status::Captured,
                                    Some(Msg::BaSelect(BaSelect::Begin(c))),
                                );
                            }
                        }
                        _ => (),
                    },
                    _ => (),
                },
                Event::Mouse(mouse::Event::ButtonReleased(bp)) => match bp {
                    mouse::Button::Left => match cursor {
                        mouse::Cursor::Available(cur) => {
                            if self.cur_moving {
                                return (
                                    iced::event::Status::Captured,
                                    Some(Msg::BaSelect(BaSelect::End(Some(c)))),
                                );
                            }
                        }
                        _ => (),
                    },
                    _ => (),
                },
                Event::Mouse(mouse::Event::CursorMoved { position: pos }) => {
                    if self.cur_moving {
                        return (
                            iced::event::Status::Captured,
                            Some(Msg::BaSelect(BaSelect::Move(c))),
                        );
                    }
                }
                _ => {}
            }
        } else if matches!(event, Event::Mouse(_)) {
            if self.cur_moving {
                return (
                    iced::event::Status::Ignored,
                    Some(Msg::BaSelect(BaSelect::End(None))),
                );
            }
        }

        match event {
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(ev)) => {
                state.show_raw = ev.control();
            }
            _ => (),
        }
        (iced::event::Status::Ignored, None)
    }
}

const VMAP_LEN: usize = 100;
impl Default for BalanceChart {
    fn default() -> Self {
        let num = 40;
        let mut measurements = HeapRb::new(num);
        // measurements.push_slice(&[2500, 2220, 2000, 2200, 2205, 1500, 3000]);
        Self {
            measurements,
            refweight: Default::default(),
            pcoupler: Default::default(),
            omit: 0,
            select: None,
            cur_moving: false,
            balance_rp: HeapRb::new(RP_LEN),
            vmaps: HeapRb::new(VMAP_LEN),
            speed: HeapRb::new(SPEED_LEN),
        }
    }
}

struct OverBtn(bool);

impl button::StyleSheet for OverBtn {
    type Style = iced::Theme;
    fn active(&self, style: &Self::Style) -> button::Appearance {
        let mut c = if self.0 {
            colors::OLIVE
        } else {
            colors::DARK_BLUE
        };
        let mut t = colors::WHITE;
        c.a = 0.3;
        t.a = 0.9;
        Appearance {
            background: Some(c.into()),
            text_color: t,
            ..Default::default()
        }
    }
    fn hovered(&self, style: &Self::Style) -> Appearance {
        let mut c = colors::OLIVE;
        c.a = 0.6;
        Appearance {
            background: Some(c.into()),
            ..self.active(style)
        }
    }
    fn pressed(&self, style: &Self::Style) -> Appearance {
        let mut c = colors::OLIVE;
        c.a = 0.8;
        Appearance {
            background: Some(c.into()),
            ..self.active(style)
        }
    }
}

impl BalanceChart {
    pub fn view(&self) -> Element<'_, Msg> {
        let sld = vertical_slider(
            0..=(self.measurements.occupied_len() as u32),
            self.omit,
            |v| Msg::Omit(v),
        );

        let btn_wd = 100.;
        let flted = row!(column([
            button(text(format!(
                "0mg={}",
                &self
                    .refweight
                    .get(&RefWeight::Num(0))
                    .map(|x| (x.floor()).numer().map(|x| *x))
                    .map_or("".to_owned(), |x| x.unwrap().to_string())
            )))
            .on_press(Msg::RefWeight(RefWeight::Num(0)))
            .width(Length::Fixed(btn_wd))
            .style(theme::Button::custom(OverBtn(
                self.refweight.contains_key(&RefWeight::Num(0))
            )))
            .into(),
            button(text(format!(
                "1mg={}",
                &self
                    .refweight
                    .get(&RefWeight::Num(1))
                    .map(|x| (x.ceil()).numer().map(|x| *x))
                    .map_or("".to_owned(), |x| x.unwrap().to_string())
            )))
            .on_press(Msg::RefWeight(RefWeight::Num(1)))
            .width(Length::Fixed(btn_wd))
            .style(theme::Button::custom(OverBtn(
                self.refweight.contains_key(&RefWeight::Num(1))
            )))
            .into(),
            button(text(format!(
                "10mg={}",
                &self
                    .refweight
                    .get(&RefWeight::Num(10))
                    .map(|x| (x.ceil()).numer().map(|x| *x))
                    .map_or("".to_owned(), |x| x.unwrap().to_string())
            )))
            .on_press(Msg::RefWeight(RefWeight::Num(10)))
            .width(Length::Fixed(btn_wd))
            .style(theme::Button::custom(OverBtn(
                self.refweight.contains_key(&RefWeight::Num(10))
            )))
            .into(),
            button(text(format!(
                "orig={}",
                &self
                    .refweight
                    .get(&RefWeight::Origin)
                    .map(|x| (x.ceil()).numer().map(|x| *x))
                    .map_or("".to_owned(), |x| x.unwrap().to_string())
            )))
            .on_press(Msg::RefWeight(RefWeight::Origin))
            .width(Length::Fixed(btn_wd))
            .style(theme::Button::custom(OverBtn(
                self.refweight.contains_key(&RefWeight::Origin)
            )))
            .into()
        ])
        .spacing(5))
        .spacing(5);
        let flted = container(flted).padding(15);
        let sld = container(sld).padding(Padding {
            left: 0.,
            right: 0.,
            top: 20.,
            bottom: 20.,
        });

        let ofs = iced_aw::floating_element::Offset { x: 30., y: 0. };
        let flt = iced_aw::floating_element(ChartWidget::new(self), flted)
            .anchor(Anchor::NorthWest)
            .offset(ofs);
        row!(flt, sld).spacing(0).into()
    }
}
