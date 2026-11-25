use std::{cell::RefCell, rc::Rc};

use gpui::{Bounds, Edges, ParentElement, Pixels, Point, Styled, canvas, px, quad, rgb, rgba};

use super::bridge::Target;

#[derive(Clone, Default)]
pub(crate) struct Picker(Rc<RefCell<Vec<Region>>>);

#[derive(Clone)]
struct Region {
    id: &'static str,
    label: &'static str,
    bounds: Bounds<Pixels>,
}

impl Picker {
    pub(crate) fn mark<E: ParentElement + Styled>(
        picker: Option<&Self>,
        element: E,
        id: &'static str,
        label: &'static str,
    ) -> E {
        let Some(picker) = picker.cloned() else {
            return element;
        };
        element.relative().child(
            canvas(
                move |bounds, window, _| {
                    let bounds = bounds.intersect(&window.content_mask().bounds);
                    if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                        picker.0.borrow_mut().push(Region { id, label, bounds });
                    }
                },
                |_, (), _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }

    fn region_at(&self, position: Point<Pixels>) -> Option<Region> {
        self.0
            .borrow()
            .iter()
            .rev()
            .filter(|region| region.bounds.contains(&position))
            .min_by(|a, b| {
                let area = |region: &Region| {
                    f32::from(region.bounds.size.width) * f32::from(region.bounds.size.height)
                };
                area(a).total_cmp(&area(b))
            })
            .cloned()
    }

    pub(super) fn target_at(&self, position: Point<Pixels>) -> Option<Target> {
        let Region { id, label, bounds } = self.region_at(position)?;
        Some(Target {
            id: id.into(),
            label: label.into(),
            bounds: [
                bounds.origin.x.into(),
                bounds.origin.y.into(),
                bounds.size.width.into(),
                bounds.size.height.into(),
            ],
        })
    }

    pub(super) fn highlight(&self) -> impl gpui::IntoElement + use<> {
        let picker = self.clone();
        canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                if let Some(region) = picker.region_at(window.mouse_position()) {
                    window.paint_quad(quad(
                        region.bounds,
                        px(3.),
                        rgba(0x2859ad22),
                        Edges::all(px(2.)),
                        rgb(0x2859ad),
                        gpui::BorderStyle::Solid,
                    ));
                }
            },
        )
        .absolute()
        .inset_0()
    }
}
