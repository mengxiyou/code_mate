//! 事件抽象(对应 pc/events.py)。把「触发」与「响应」解耦:
//! 不同来源/平台产生**同一组逻辑事件**(LID_CLOSED / LID_OPENED / BUTTON_NEXT),
//! 经公共 EventBus 分发;订阅者(host)只认逻辑事件、不关心来自哪个平台或设备。
//!
//! EventBus:线程安全发布订阅,handler 在 publish 的调用线程内**同步**执行;
//! 单个 handler panic 不影响其它订阅者(catch_unwind 吞掉,对齐 Python 的 try/except)。
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

/// 逻辑事件类型(对齐 pc/events.py 的字符串常量)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventType {
    LidClosed,
    LidOpened,
    ButtonNext,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: EventType,
    pub data: Option<String>,
}

impl Event {
    pub fn new(kind: EventType, data: Option<String>) -> Self {
        Event { kind, data }
    }
}

type Handler = Arc<dyn Fn(&Event) + Send + Sync>;

/// 线程安全发布订阅。subscribe/publish 跨线程安全;publish 同步逐个调 handler。
#[derive(Default)]
pub struct EventBus {
    subs: Mutex<Vec<Handler>>,
}

impl EventBus {
    pub fn new() -> Self {
        EventBus { subs: Mutex::new(Vec::new()) }
    }

    pub fn subscribe(&self, handler: Handler) {
        self.subs.lock().unwrap().push(handler);
    }

    pub fn publish(&self, ev: Event) {
        // 复制订阅者列表后释放锁,再逐个调用(handler 内可能再 publish/subscribe,避免重入死锁)
        let subs: Vec<Handler> = self.subs.lock().unwrap().clone();
        for h in subs {
            // 单个 handler panic 不能掀翻整条总线(对齐 Python 的 except: pass)
            let _ = catch_unwind(AssertUnwindSafe(|| h(&ev)));
        }
    }
}

/// 触发源:start()/stop() 生命周期 + 向总线 emit(子类实现具体平台/设备机制)。
/// 对齐 pc/events.EventSource;具体实现见 lid_watch::LidEventSource。
pub trait EventSource {
    fn start(&mut self) {}
    fn stop(&mut self) {}
}
