use super::*;

fn name(text: &str) -> SessionName {
    text.parse().unwrap()
}

fn full(places: &mut Places, names: &[&str]) {
    for text in names {
        places.arrive(name(text)).unwrap();
    }
}

#[test]
fn a_new_session_takes_the_focus_slot_first_then_the_rail_in_order() {
    let mut places = Places::new(4);
    let arrivals: Vec<_> = ["a", "b", "c", "d"]
        .into_iter()
        .map(|text| places.arrive(name(text)).unwrap())
        .collect();
    assert_eq!(
        arrivals,
        vec![
            Arrival {
                place: Place::Focus,
                replaced: None
            },
            Arrival {
                place: Place::Rail(0),
                replaced: None
            },
            Arrival {
                place: Place::Rail(1),
                replaced: None
            },
            Arrival {
                place: Place::Rail(2),
                replaced: None
            },
        ]
    );
}

#[test]
fn one_session_has_no_rail_and_a_second_brings_it() {
    let mut places = Places::new(4);
    places.arrive(name("a")).unwrap();
    assert!(!places.has_rail());
    places.arrive(name("b")).unwrap();
    assert!(places.has_rail());
}

#[test]
fn a_new_session_takes_an_empty_place_before_replacing_an_ended_card() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b"]);
    places.end(&name("a"));
    assert_eq!(
        places.arrive(name("c")),
        Some(Arrival {
            place: Place::Rail(1),
            replaced: None
        })
    );
    assert_eq!(places.place(&name("a")), Some(Place::Focus));
}

#[test]
fn with_no_empty_place_a_new_session_replaces_the_oldest_ended_card() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b", "c", "d"]);
    places.end(&name("c"));
    places.end(&name("a"));
    assert_eq!(
        places.vacancy(),
        Some(Arrival {
            place: Place::Rail(1),
            replaced: Some(name("c"))
        })
    );
    assert_eq!(places.place(&name("c")), Some(Place::Rail(1)));
    assert_eq!(
        places.arrive(name("e")),
        Some(Arrival {
            place: Place::Rail(1),
            replaced: Some(name("c"))
        })
    );
    assert_eq!(places.place(&name("c")), None);
    assert_eq!(places.place(&name("e")), Some(Place::Rail(1)));
}

#[test]
fn every_place_holding_a_running_session_leaves_no_room() {
    let mut places = Places::new(2);
    full(&mut places, &["a", "b"]);
    assert_eq!(places.vacancy(), None);
    assert_eq!(places.arrive(name("c")), None);
    assert_eq!(places.place(&name("c")), None);
}

#[test]
fn showing_a_rail_session_swaps_it_with_the_focused_one() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b", "c"]);
    assert!(places.show(&name("c")));
    assert_eq!(places.place(&name("c")), Some(Place::Focus));
    assert_eq!(places.place(&name("a")), Some(Place::Rail(1)));
    assert_eq!(places.place(&name("b")), Some(Place::Rail(0)));
}

#[test]
fn showing_the_focused_or_an_unknown_session_changes_nothing() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b"]);
    assert!(!places.show(&name("a")));
    assert!(!places.show(&name("z")));
    assert_eq!(places.place(&name("a")), Some(Place::Focus));
    assert_eq!(places.place(&name("b")), Some(Place::Rail(0)));
}

#[test]
fn an_ended_card_in_the_rail_can_be_shown() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b"]);
    places.end(&name("b"));
    assert!(places.show(&name("b")));
    assert_eq!(places.place(&name("b")), Some(Place::Focus));
}

#[test]
fn when_the_focus_slot_empties_the_oldest_running_rail_session_moves_in() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b", "c", "d"]);
    places.end(&name("b"));
    places.leave(&name("a"));
    assert_eq!(places.place(&name("c")), Some(Place::Focus));
    assert_eq!(places.place(&name("b")), Some(Place::Rail(0)));
    assert_eq!(places.place(&name("d")), Some(Place::Rail(1)));
}

#[test]
fn an_ended_card_does_not_move_into_an_empty_focus_slot() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b"]);
    places.end(&name("b"));
    places.leave(&name("a"));
    assert_eq!(places.place(&name("b")), Some(Place::Rail(0)));
    assert_eq!(
        places.arrive(name("c")),
        Some(Arrival {
            place: Place::Focus,
            replaced: None
        })
    );
}

#[test]
fn a_session_leaving_the_rail_lets_the_cards_below_move_up() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b", "c", "d"]);
    places.leave(&name("b"));
    assert_eq!(places.place(&name("c")), Some(Place::Rail(0)));
    assert_eq!(places.place(&name("d")), Some(Place::Rail(1)));
    assert_eq!(
        places.arrive(name("e")),
        Some(Arrival {
            place: Place::Rail(2),
            replaced: None
        })
    );
}

#[test]
fn a_swapped_in_session_counts_its_age_from_when_it_opened() {
    let mut places = Places::new(4);
    full(&mut places, &["a", "b", "c"]);
    assert!(places.show(&name("c")));
    places.leave(&name("c"));
    assert_eq!(places.place(&name("a")), Some(Place::Focus));
    assert_eq!(places.place(&name("b")), Some(Place::Rail(0)));
}
