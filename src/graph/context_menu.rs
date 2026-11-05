use std::collections::BTreeMap;
use std::fmt::Debug;
use std::marker::PhantomData;

use egui::{Pos2, Ui};

use crate::graph::node::NodeBlueprintDefinition;
use crate::graph::node::port::{ConstantInputType, PortTypeInfo};
use crate::graph::painter::Painter;
use crate::graph::painter::style::UNZOOMED;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct MenuCategory<K> {
    pub(crate) title: String,
    pub(crate) subcategories: BTreeMap<String, MenuCategory<K>>,
    pub(crate) elements: BTreeMap<String, K>,
}

impl<K> Default for MenuCategory<K> {
    fn default() -> Self {
        Self {
            title: "".to_string(),
            subcategories: BTreeMap::new(),
            elements: BTreeMap::new(),
        }
    }
}

pub(crate) struct ContextMenu<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    menu: MenuCategory<K>,
    open_submenu: Option<String>,
    position: Option<Pos2>,
    _marker: PhantomData<P>,
    _marker_d: PhantomData<D>,
}

impl<P, D, K> Default for ContextMenu<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    fn default() -> ContextMenu<P, D, K> {
        Self {
            menu: Default::default(),
            open_submenu: None,
            position: None,
            _marker: PhantomData,
            _marker_d: PhantomData,
        }
    }
}

impl<P, D, K> ContextMenu<P, D, K>
where
    P: PortTypeInfo + Eq + Copy + Clone + Debug,
    D: PartialEq + Clone + Debug + for<'a> From<&'a ConstantInputType> + Send + Sync + 'static,
    for<'a> &'a D: TryInto<String>,
    K: NodeBlueprintDefinition<P, D>,
{
    pub(crate) fn register_node_blueprint(&mut self, blueprint: K) {
        for category in blueprint.categories().iter() {
            if category.is_empty() {
                self.menu
                    .elements
                    .insert(blueprint.name().to_string(), blueprint);
            } else {
                if !self.menu.subcategories.contains_key(*category) {
                    self.menu.subcategories.insert(
                        category.to_string(),
                        MenuCategory {
                            title: category.to_string(),
                            subcategories: Default::default(),
                            elements: Default::default(),
                        },
                    );
                }

                self.menu
                    .subcategories
                    .get_mut(*category)
                    .unwrap()
                    .elements
                    .insert(blueprint.name().to_string(), blueprint);
            }
        }
    }

    pub(super) fn show(&mut self, ui: &Ui, painter: &Painter) -> Option<K> {
        let mut spawned_node: Option<K> = None;

        if let Some(position) = self.position {
            painter
                .show_context_menu_area("node_context_menu", position)
                .show(ui, |ui| {
                    painter.show_scoped(ui, UNZOOMED, |ui| {
                        painter.show_context_menu_frame().show(ui, |ui| {
                            for title in self.menu.subcategories.keys() {
                                let response = ui.add(painter.show_context_menu_button(title));

                                if response.clicked() {
                                    self.open_submenu = Some(title.clone());
                                }
                            }

                            for (title, element) in self.menu.elements.iter() {
                                if ui.add(painter.show_context_menu_button(title)).clicked() {
                                    spawned_node = Some(*element);
                                    ui.close();
                                }
                            }
                        });
                    });
                });
            if self.open_submenu.is_some() {
                let category = &self.menu.subcategories[self.open_submenu.as_ref().unwrap()];

                painter
                    .show_context_submenu_area("node_context_submenu", position)
                    .show(ui, |ui| {
                        painter.show_context_menu_frame().show(ui, |ui| {
                            for (title, element) in category.elements.iter() {
                                if ui.add(painter.show_context_menu_button(title)).clicked() {
                                    spawned_node = Some(*element);
                                    self.open_submenu = None;
                                }
                            }
                        });
                    });
            }
        }

        spawned_node
    }

    pub fn open_at_position(&mut self, position: Pos2) {
        self.position = Some(position);
        self.open_submenu = None;
    }

    pub fn close(&mut self) {
        self.position = None;
        self.open_submenu = None;
    }
}
