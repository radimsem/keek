use crate::session_folder::SessionName;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    Focus,
    Rail(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub place: Place,
    pub replaced: Option<SessionName>,
}

struct Occupant {
    name: SessionName,
    arrived: usize,
    ended: Option<usize>,
}

pub struct Places {
    capacity: usize,
    focus: Option<Occupant>,
    rail: Vec<Occupant>,
    order: usize,
}

impl Places {
    pub fn new(places: usize) -> Self {
        Self {
            capacity: places,
            focus: None,
            rail: Vec::new(),
            order: 0,
        }
    }

    pub fn vacancy(&self) -> Option<Arrival> {
        if self.capacity == 0 {
            return None;
        }
        let place = if self.focus.is_none() {
            Place::Focus
        } else if self.rail.len() < self.capacity - 1 {
            Place::Rail(self.rail.len())
        } else {
            return self
                .entries()
                .filter_map(|(occupant, place)| Some((occupant.ended?, occupant, place)))
                .min_by_key(|(ended, _, _)| *ended)
                .map(|(_, occupant, place)| Arrival {
                    place,
                    replaced: Some(occupant.name.clone()),
                });
        };
        Some(Arrival {
            place,
            replaced: None,
        })
    }

    pub fn arrive(&mut self, name: SessionName) -> Option<Arrival> {
        let arrival = self.vacancy()?;
        let occupant = Occupant {
            name,
            arrived: self.order,
            ended: None,
        };
        self.order += 1;
        match arrival.place {
            Place::Focus => self.focus = Some(occupant),
            Place::Rail(index) => {
                if let Some(previous) = self.rail.get_mut(index) {
                    *previous = occupant;
                } else {
                    self.rail.push(occupant);
                }
            }
        }
        Some(arrival)
    }

    pub fn end(&mut self, name: &SessionName) {
        let occupant = match self.place(name) {
            Some(Place::Focus) => self.focus.as_mut(),
            Some(Place::Rail(index)) => self.rail.get_mut(index),
            None => None,
        };
        if let Some(occupant) = occupant
            && occupant.ended.is_none()
        {
            occupant.ended = Some(self.order);
            self.order += 1;
        }
    }

    pub fn leave(&mut self, name: &SessionName) {
        match self.place(name) {
            Some(Place::Focus) => self.focus = None,
            Some(Place::Rail(index)) => {
                self.rail.remove(index);
            }
            None => return,
        }
        if self.focus.is_none()
            && let Some((index, _)) = self
                .rail
                .iter()
                .enumerate()
                .filter(|(_, occupant)| occupant.ended.is_none())
                .min_by_key(|(_, occupant)| occupant.arrived)
        {
            self.focus = Some(self.rail.remove(index));
        }
    }

    pub fn show(&mut self, name: &SessionName) -> bool {
        let Some(Place::Rail(index)) = self.place(name) else {
            return false;
        };
        if let Some(focus) = &mut self.focus {
            std::mem::swap(focus, &mut self.rail[index]);
        } else {
            self.focus = Some(self.rail.remove(index));
        }
        true
    }

    pub fn place(&self, name: &SessionName) -> Option<Place> {
        self.occupants()
            .find_map(|(occupant, place)| (occupant == name).then_some(place))
    }

    pub fn has_rail(&self) -> bool {
        !self.rail.is_empty()
    }

    pub fn occupants(&self) -> impl Iterator<Item = (&SessionName, Place)> {
        self.entries()
            .map(|(occupant, place)| (&occupant.name, place))
    }

    fn entries(&self) -> impl Iterator<Item = (&Occupant, Place)> {
        self.focus
            .iter()
            .map(|occupant| (occupant, Place::Focus))
            .chain(
                self.rail
                    .iter()
                    .enumerate()
                    .map(|(index, occupant)| (occupant, Place::Rail(index))),
            )
    }
}

#[cfg(test)]
#[path = "places_tests.rs"]
mod tests;
