use bevy::{ecs::world::Command, prelude::*};

pub struct FireEvent<E: Event> {
    event: E,
}

impl<E: Event> Command for FireEvent<E> {
    fn apply(self, world: &mut World) {
        let mut events = world.resource_mut::<Events<E>>();
        events.send(self.event);
    }
}

pub trait FireEventEx {
    fn fire_event<E: Event>(&mut self, e: E) -> &mut Self;
}

impl FireEventEx for Commands<'_, '_> {
    fn fire_event<E: Event>(&mut self, event: E) -> &mut Self {
        self.add(FireEvent { event });
        self
    }
}
