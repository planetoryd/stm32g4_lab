use std::{collections::HashSet, fmt::format};

use crate::*;
use bittle::prelude::*;
use common::BALANCE_BYTES;
use fraction::Fraction;
use iced::{
    advanced::graphics::{core::event, text::cosmic_text::rustybuzz::ttf_parser::Width},
    mouse,
    theme::{self, Container},
    Point, Size, Vector,
};
use iced_aw::{
    floating_element::{self, Anchor},
    style::colors,
};
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
}

pub static SELECTED_POINTS: RwLock<Option<HashSet<(usize, u32)>>> = RwLock::const_new(None);

#[test]
fn frint() {
    let frac = Fraction::new(5u32, 2u32);
    let k = frac.floor();
    dbg!(&k);
}

impl Chart<Msg> for BalanceChart {
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
        let refmin = self
            .refweight
            .get(&RefWeight::Num(0))
            .map(|x| *x.floor().numer().unwrap() as u32);
        let refmax = self
            .refweight
            .get(&RefWeight::Num(10))
            .map(|x| *x.ceil().numer().unwrap() as u32);
        let ymin = refmin.or(vymin).or(Some(1000)).unwrap();
        let ymax = refmax.or(vymax).or(Some(4095)).unwrap();

        let mut cx = cb
            .x_label_area_size(20)
            .y_label_area_size(30)
            .margin(10)
            .build_cartesian_2d(0usize..self.measurements.capacity().get(), ymin..ymax)
            .unwrap();
        cx.configure_mesh().draw().unwrap();
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
        (iced::event::Status::Ignored, None)
    }
}

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
        let sel = SELECTED_POINTS.blocking_read();
        let tx = if let Some(ref inn) = *sel {
            if inn.len() > 0 {
                let avg = inn.iter().map(|x| x.1 as usize).sum::<usize>() / inn.len();
                format!("avg={}", avg)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let flted = container(
            row!(
                column([text(tx).into()]).padding(5),
                column([
                    button("0mg")
                        .on_press(Msg::RefWeight(RefWeight::Num(0)))
                        .width(Length::Fixed(60.))
                        .style(if self.refweight.contains_key(&RefWeight::Num(0)) {
                            theme::Button::Positive
                        } else {
                            theme::Button::Secondary
                        })
                        .into(),
                    button("1mg")
                        .on_press(Msg::RefWeight(RefWeight::Num(1)))
                        .width(Length::Fixed(60.))
                        .style(if self.refweight.contains_key(&RefWeight::Num(1)) {
                            theme::Button::Positive
                        } else {
                            theme::Button::Secondary
                        })
                        .into(),
                    button("10mg")
                        .on_press(Msg::RefWeight(RefWeight::Num(10)))
                        .width(Length::Fixed(60.))
                        .style(if self.refweight.contains_key(&RefWeight::Num(10)) {
                            theme::Button::Positive
                        } else {
                            theme::Button::Secondary
                        })
                        .into(),
                    button("offset")
                        .on_press(Msg::RefWeight(RefWeight::Origin))
                        .width(Length::Fixed(60.))
                        .style(if self.refweight.contains_key(&RefWeight::Origin) {
                            theme::Button::Positive
                        } else {
                            theme::Button::Secondary
                        })
                        .into()
                ])
                .spacing(5),
            )
            .spacing(5),
        )
        .padding(20);
        let sld = container(sld).padding(Padding {
            left: 0.,
            right: 0.,
            top: 20.,
            bottom: 20.,
        });
        let flt =
            iced_aw::floating_element(ChartWidget::new(self), flted).anchor(Anchor::NorthEast);
        row!(flt, sld).spacing(0).into()
    }
}
