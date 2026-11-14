//! Connects the local calendar file to the island.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::App;
use crate::calendar::{self, AgendaStatus, CalendarFile};
use crate::dialogs;
use crate::ui::{AgendaEntry, AppActions, AppState, CalendarState, Page};

/// Events shown in the agenda page.
const AGENDA_LIMIT: usize = 6;

impl App {
    pub(super) fn connect_calendar(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.island.global::<AppActions>().on_choose_calendar(move || {
            let weak = weak.clone();
            // The dialog runs a modal loop, which must not start inside UI event dispatch.
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(app) = weak.upgrade() {
                    App::choose_calendar(&app);
                }
            });
        });
    }

    fn choose_calendar(app: &Rc<Self>) {
        App::set_expanded(app, false);
        let Some(path) = dialogs::pick_calendar_file() else { return };
        App::open_calendar(app, &path);
        App::set_expanded(app, true);
        app.island.global::<AppState>().set_page(Page::Calendar);
    }

    pub(super) fn open_calendar(app: &Rc<Self>, path: &Path) {
        app.calendar.replace(Some(CalendarFile::open(path)));
        app.refresh_agenda();
    }

    /// Rebuilds the agenda. Called when the island expands and once a minute while it is
    /// expanded; the file itself is only re-read when its modification time changes.
    pub(super) fn refresh_agenda(&self) {
        if !self.expanded() {
            return;
        }
        let state = self.island.global::<AppState>();
        let mut calendar = self.calendar.borrow_mut();
        let Some(file) = calendar.as_mut() else {
            state.set_calendar_state(CalendarState::NotConfigured);
            state.set_agenda(ModelRc::default());
            return;
        };
        file.refresh();
        let agenda = file.agenda(&self.zones, calendar::now_local(), self.h24.get(), AGENDA_LIMIT);
        let source = file.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        state.set_calendar_source(source.into());
        match agenda.status {
            AgendaStatus::Ready => {
                state.set_calendar_state(CalendarState::Ready);
                state.set_calendar_message("".into());
            }
            AgendaStatus::Unavailable(message) => {
                state.set_calendar_state(CalendarState::Unavailable);
                state.set_calendar_message(message.into());
            }
        }
        let entries: Vec<AgendaEntry> = agenda
            .items
            .into_iter()
            .map(|item| AgendaEntry {
                title: item.title.into(),
                location: item.location.into(),
                day: item.day.into(),
                time: item.time.into(),
                now: item.now,
            })
            .collect();
        state.set_agenda(ModelRc::new(VecModel::from(entries)));
    }
}
