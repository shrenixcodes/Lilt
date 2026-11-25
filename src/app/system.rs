//! Feeds CPU, memory and network readings to the island while they are visible.

use std::rc::Rc;

use slint::ComponentHandle;

use super::App;
use crate::system::{self, SamplingPolicy};
use crate::ui::AppState;

#[derive(Clone, Copy)]
enum Metric {
    Cpu,
    Memory,
    Network,
}

impl App {
    /// Starts, stops or re-times the sampling timers to match the island's state. A
    /// metric that becomes visible is sampled immediately; the first CPU reading after a
    /// pause is the average over the pause, and the next follows within the interval.
    pub(super) fn update_sampling(app: &Rc<Self>) {
        let settings = app.settings.borrow();
        let active = settings.widgets.system && app.island_visible();
        let policy = if app.system_monitor.borrow().is_some() {
            SamplingPolicy::for_state(active, app.expanded(), settings.performance.low_power)
        } else {
            SamplingPolicy::OFF
        };
        drop(settings);
        let previous = app.sampling.replace(policy);
        for (metric, interval, before) in [
            (Metric::Cpu, policy.cpu, previous.cpu),
            (Metric::Memory, policy.memory, previous.memory),
            (Metric::Network, policy.network, previous.network),
        ] {
            if interval == before {
                continue;
            }
            let timer = app.metric_timer(metric);
            match interval {
                Some(interval) => {
                    App::sample(app, metric);
                    let weak = Rc::downgrade(app);
                    timer.start(slint::TimerMode::Repeated, interval, move || {
                        if let Some(app) = weak.upgrade() {
                            App::sample(&app, metric);
                        }
                    });
                }
                None => timer.stop(),
            }
        }
    }

    fn metric_timer(&self, metric: Metric) -> &slint::Timer {
        match metric {
            Metric::Cpu => &self.cpu_timer,
            Metric::Memory => &self.memory_timer,
            Metric::Network => &self.network_timer,
        }
    }

    fn sample(app: &Rc<Self>, metric: Metric) {
        let mut monitor = app.system_monitor.borrow_mut();
        let Some(monitor) = monitor.as_mut() else { return };
        let state = app.island.global::<AppState>();
        let mut info = state.get_system();
        match metric {
            Metric::Cpu => {
                let cpu = monitor.cpu();
                info.cpu = cpu / 100.0;
                info.cpu_text = format!("{cpu:.0}%").into();
            }
            Metric::Memory => {
                let memory = monitor.memory();
                info.memory = memory.used_fraction();
                info.memory_text = format!("{:.0}%", memory.used_fraction() * 100.0).into();
                info.memory_detail =
                    format!("{} of {} in use", system::format_gb(memory.used()), system::format_gb(memory.total))
                        .into();
            }
            Metric::Network => {
                if let Some(rates) = monitor.network() {
                    info.down_text = system::format_rate(rates.down).into();
                    info.up_text = system::format_rate(rates.up).into();
                }
            }
        }
        info.ready = !info.cpu_text.is_empty() && !info.memory_text.is_empty();
        state.set_system(info);
    }
}
