use std::collections::HashMap;

pub struct EventEmitter {
    handlers: HashMap<String, Box<dyn Fn(&str) + Send + Sync + 'static>>,
}

impl EventEmitter {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn on<F: Fn(&str) + Send + Sync + 'static>(&mut self, event: &str, handler: F) {
        self.handlers.insert(event.to_string(), Box::new(handler));
    }

    pub fn emit(&self, event: &str, data: &str) {
        if let Some(handler) = self.handlers.get(event) {
            handler(data);
        }
    }

    pub fn off(&mut self, event: &str) {
        self.handlers.remove(event);
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::EventEmitter;

    #[test]
    fn emits_to_registered_handler() {
        let mut emitter = EventEmitter::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&received);

        emitter.on("hello", move |data| {
            captured.lock().unwrap().push(data.to_string());
        });

        emitter.emit("hello", "world");

        assert_eq!(*received.lock().unwrap(), vec!["world".to_string()]);
    }

    #[test]
    fn off_removes_handler() {
        let mut emitter = EventEmitter::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&received);

        emitter.on("hello", move |data| {
            captured.lock().unwrap().push(data.to_string());
        });
        emitter.off("hello");

        emitter.emit("hello", "world");

        assert!(received.lock().unwrap().is_empty());
    }

    #[test]
    fn replacing_handler_uses_latest_registration() {
        let mut emitter = EventEmitter::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let first = Arc::clone(&received);
        let second = Arc::clone(&received);

        emitter.on("hello", move |data| {
            first.lock().unwrap().push(format!("first:{data}"));
        });
        emitter.on("hello", move |data| {
            second.lock().unwrap().push(format!("second:{data}"));
        });

        emitter.emit("hello", "world");

        assert_eq!(*received.lock().unwrap(), vec!["second:world".to_string()]);
    }
}
