use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

struct QueueWake {
    queue: Arc<Mutex<VecDeque<usize>>>,
    task_id: usize,
}

impl Wake for QueueWake {
    fn wake(self: Arc<Self>) {
        self.queue.lock().unwrap().push_back(self.task_id);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.queue.lock().unwrap().push_back(self.task_id);
    }
}

fn simple_waker(queue: Arc<Mutex<VecDeque<usize>>>, task_id: usize) -> Waker {
    Waker::from(Arc::new(QueueWake { queue, task_id }))
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let queue = Arc::new(Mutex::new(VecDeque::from([0usize])));
    let waker = simple_waker(queue.clone(), 0);
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);

    loop {
        if queue.lock().unwrap().pop_front().is_some() {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }
}

pub struct Executor {
    tasks: VecDeque<Pin<Box<dyn Future<Output = ()> + 'static>>>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            tasks: VecDeque::new(),
        }
    }

    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.tasks.push_back(Box::pin(future));
    }

    pub fn drain(&mut self) {
        while let Some(mut task) = self.tasks.pop_front() {
            block_on(async move {
                task.as_mut().await;
            });
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{block_on, Executor};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::task::{Context, Poll};
    use std::future::Future;
    use std::pin::Pin;

    #[test]
    fn block_on_runs_async_block_to_completion() {
        let value = block_on(async { 1 + 2 });
        assert_eq!(value, 3);
    }

    #[test]
    fn block_on_polls_pending_future_until_ready() {
        let polled = Rc::new(Cell::new(0));
        let polled2 = polled.clone();

        let value = block_on(async move {
            struct TwoPollFuture {
                polled: Rc<Cell<usize>>,
            }

            impl Future for TwoPollFuture {
                type Output = usize;

                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                    let n = self.polled.get();
                    self.polled.set(n + 1);
                    if n == 0 {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    } else {
                        Poll::Ready(7)
                    }
                }
            }

            TwoPollFuture { polled: polled2 }.await
        });

        assert_eq!(value, 7);
        assert_eq!(polled.get(), 2);
    }

    #[test]
    fn executor_spawns_and_drains_tasks() {
        let hits = Rc::new(Cell::new(0));
        let mut executor = Executor::new();

        for _ in 0..3 {
            let hits = hits.clone();
            executor.spawn(async move {
                hits.set(hits.get() + 1);
            });
        }

        executor.drain();
        assert_eq!(hits.get(), 3);
        assert!(executor.tasks.is_empty());
    }
}
