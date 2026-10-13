use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEvent {
    pub event_type: String,
    pub properties: serde_json::Value,
}

pub struct BusEventDef {
    pub event_type: &'static str,
}

impl BusEventDef {
    pub const fn new(event_type: &'static str) -> Self {
        Self { event_type }
    }
}

type BoxedCallback = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;

struct Subscription {
    id: u64,
    callback: BoxedCallback,
}

pub struct Bus {
    next_id: Arc<RwLock<u64>>,
    subscribers: Arc<RwLock<HashMap<String, Vec<Subscription>>>>,
    wildcard_subscribers: Arc<RwLock<Vec<Subscription>>>,
    tx: broadcast::Sender<BusEvent>,
}

impl Bus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            next_id: Arc::new(RwLock::new(0)),
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            wildcard_subscribers: Arc::new(RwLock::new(Vec::new())),
            tx,
        }
    }

    pub async fn publish(&self, def: &BusEventDef, properties: serde_json::Value) {
        tracing::info!(event_type = def.event_type, "publishing event");

        let event = BusEvent {
            event_type: def.event_type.to_string(),
            properties: properties.clone(),
        };

        let _ = self.tx.send(event.clone());

        let subscribers = self.subscribers.read().await;
        if let Some(subs) = subscribers.get(def.event_type) {
            for sub in subs {
                (sub.callback)(def.event_type, properties.clone());
            }
        }

        let wildcard = self.wildcard_subscribers.read().await;
        for sub in wildcard.iter() {
            (sub.callback)(def.event_type, properties.clone());
        }
    }

    pub async fn subscribe<F>(&self, def: &BusEventDef, callback: F) -> u64
    where
        F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
    {
        let id = {
            let mut next = self.next_id.write().await;
            *next += 1;
            *next
        };

        let mut subscribers = self.subscribers.write().await;
        let subs = subscribers
            .entry(def.event_type.to_string())
            .or_insert_with(Vec::new);
        subs.push(Subscription {
            id,
            callback: Box::new(callback),
        });

        id
    }

    pub async fn unsubscribe(&self, event_type: &str, id: u64) {
        let mut subscribers = self.subscribers.write().await;
        if let Some(subs) = subscribers.get_mut(event_type) {
            subs.retain(|s| s.id != id);
        }
    }

    pub async fn subscribe_all<F>(&self, callback: F) -> u64
    where
        F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
    {
        let id = {
            let mut next = self.next_id.write().await;
            *next += 1;
            *next
        };

        let mut wildcard = self.wildcard_subscribers.write().await;
        wildcard.push(Subscription {
            id,
            callback: Box::new(callback),
        });

        id
    }

    pub async fn unsubscribe_all(&self, id: u64) {
        let mut wildcard = self.wildcard_subscribers.write().await;
        wildcard.retain(|s| s.id != id);
    }

    pub fn subscribe_channel(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

pub fn define_event(event_type: &'static str) -> BusEventDef {
    BusEventDef::new(event_type)
}
