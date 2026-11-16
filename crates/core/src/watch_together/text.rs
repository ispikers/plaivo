use super::media::Media;
use super::party::{Connection, EndReason, Event, PartyError, PartyState};
use super::protocol::{Match, MatchState, PlayerState};

impl PartyError {
    pub fn text(&self) -> Option<&'static str> {
        let starting = matches!(self.re.as_deref(), Some("create" | "join"));
        Some(match self.code.as_str() {
            "cancelled" => return None,
            "rate_limited" if starting => {
                "Too many rooms from your network right now. Try again in a few minutes."
            }
            "rate_limited" => "That was too fast. Wait a moment and try again.",
            "client_blocked" => "This Watch Together server doesn't accept Plaivo.",
            "unsupported" => "This Watch Together server doesn't support that.",
            "invalid_code" => "That isn't a valid room code.",
            "room_not_found" => "No room with that code.",
            "room_full" => "That room is full.",
            "room_locked" | "locked" => "That room is locked.",
            "relay_full" => "The Watch Together servers are busy. Try again soon.",
            "unreachable" => "Couldn't reach the Watch Together server.",
            "forbidden" | "not_host" => "Only the host can do that in this room.",
            "not_in_room" => "You're not connected to the room right now.",
            "already_in_room" => "You're already in a room.",
            _ => "Watch Together ran into a problem.",
        })
    }
}

impl EndReason {
    pub fn text(self) -> Option<&'static str> {
        Some(match self {
            EndReason::Left => return None,
            EndReason::Ended => "The room has ended.",
            EndReason::Kicked => "You were removed from the room.",
            EndReason::Expired => "The room expired.",
            EndReason::Replaced => "You joined this room from another device.",
            EndReason::Lost => "Lost the connection to the room.",
        })
    }
}

impl PartyState {
    pub fn waiting_text(&self) -> Option<String> {
        let room = self.room.as_ref()?;
        let names: Vec<&str> = room
            .waiting_on
            .iter()
            .filter(|id| Some(id.as_str()) != self.peer_id.as_deref())
            .map(|id| room.participant(id).map_or("someone", |p| p.name.as_str()))
            .collect();
        (!names.is_empty()).then(|| format!("Waiting for {}", names.join(", ")))
    }

    pub fn sync_text(&self) -> &'static str {
        match (self.connection, self.match_.map(|m| m.state)) {
            (Connection::Reconnecting, _) => "Reconnecting",
            (_, Some(MatchState::Unavailable)) => "Not in your libraries",
            _ => sync_label(self.player, self.match_, true),
        }
    }
}

impl Media {
    pub fn display_title(&self) -> String {
        match &self.series {
            Some(series) => crate::join(&[
                series.title.clone(),
                self.season
                    .zip(self.episode)
                    .map(|(s, e)| format!("S{s} · E{e}"))
                    .unwrap_or_default(),
                self.title.clone(),
            ]),
            None => self.title.clone(),
        }
    }

    pub fn not_found_text(&self) -> String {
        let title = self.series.as_ref().map_or(&self.title, |s| &s.title);
        format!("{title} isn't in your libraries.")
    }
}

pub fn mismatch_text(delta_ms: i64) -> String {
    let len = if delta_ms > 0 { "longer" } else { "shorter" };
    format!(
        "Your copy is {:.1} s {len} than the room's.",
        delta_ms.abs() as f64 / 1000.0
    )
}

impl Event {
    pub fn text(&self, s: &PartyState) -> Option<String> {
        match self {
            Event::Action { by_me: true, .. } => None,
            Event::Action { kind, by, .. } => {
                let what = match kind.as_str() {
                    "pause" => "paused",
                    "play" => "resumed",
                    "seek" => "skipped",
                    "rate" => "changed the speed",
                    "media" => "changed what's playing",
                    _ => return None,
                };
                let who = by
                    .as_deref()
                    .and_then(|id| s.room.as_ref()?.participant(id))
                    .map_or("Someone", |p| p.name.as_str());
                Some(format!("{who} {what}"))
            }
            Event::ParticipantJoined(p) => Some(format!("{} joined", p.name)),
            Event::ParticipantLeft(p) => Some(format!("{} left", p.name)),
            Event::Error(e) if matches!(e.re.as_deref(), None | Some("status" | "ping")) => None,
            Event::Error(e) | Event::Failed(e) => e.text().map(str::to_owned),
            Event::Ended { reason, .. } => reason.text().map(str::to_owned),
        }
    }
}

pub fn sync_label(player: PlayerState, m: Option<Match>, connected: bool) -> &'static str {
    match (connected, m.map(|m| m.state), player) {
        (false, _, _) => "Reconnecting",
        (_, None, _) => "Nothing playing",
        (_, Some(MatchState::Searching), _) => "Finding the video",
        (_, Some(MatchState::Unavailable), _) => "Not in their libraries",
        (_, _, PlayerState::Idle) => "Not watching",
        (_, _, PlayerState::Loading) => "Loading",
        (_, _, PlayerState::Buffering) => "Buffering",
        (_, Some(MatchState::Mismatched), PlayerState::Ready) => "In sync · different cut",
        (_, Some(MatchState::Matched), PlayerState::Ready) => "In sync",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_their_cause_and_skip_automatic_messages() {
        let err = |code: &str, re: Option<&str>| PartyError {
            code: code.into(),
            message: String::new(),
            re: re.map(Into::into),
        };
        let s = PartyState::default();
        assert_ne!(
            err("rate_limited", Some("join")).text(),
            err("rate_limited", Some("seek")).text()
        );
        for re in [None, Some("status"), Some("ping")] {
            assert_eq!(Event::Error(err("rate_limited", re)).text(&s), None);
        }
        assert!(
            Event::Error(err("forbidden", Some("seek")))
                .text(&s)
                .is_some()
        );
    }

    #[test]
    fn every_error_core_raises_has_its_own_text() {
        let fallback = PartyError::local("maintenance").text();
        assert_eq!(fallback, Some("Watch Together ran into a problem."));
        assert_eq!(PartyError::local("invalid_message").text(), fallback);
        assert_eq!(PartyError::local("cancelled").text(), None);
        for code in [
            "invalid_code",
            "room_not_found",
            "room_full",
            "room_locked",
            "locked",
            "relay_full",
            "unreachable",
            "forbidden",
            "not_host",
            "not_in_room",
            "already_in_room",
            "rate_limited",
            "client_blocked",
            "unsupported",
        ] {
            let text = PartyError::local(code).text();
            assert!(text.is_some() && text != fallback, "{code}");
        }
    }

    #[test]
    fn room_media_and_own_sync_read_the_same_everywhere() {
        let mut media: Media = serde_json::from_value(serde_json::json!({
            "kind": "episode", "title": "Pilot", "season": 1, "episode": 3,
            "series": {"title": "Show"}, "durationMs": 1000
        }))
        .unwrap();
        assert_eq!(media.display_title(), "Show · S1 · E3 · Pilot");
        assert_eq!(media.not_found_text(), "Show isn't in your libraries.");
        media.series = None;
        assert_eq!(media.display_title(), "Pilot");
        assert_eq!(
            mismatch_text(-1500),
            "Your copy is 1.5 s shorter than the room's."
        );

        let at = |state, offset_ms, connection| PartyState {
            connection,
            media: Some(media.clone()),
            match_: Some(Match {
                state,
                delta_ms: None,
            }),
            offset_ms,
            player: PlayerState::Ready,
            ..Default::default()
        };
        let live = Connection::Connected;
        assert_eq!(
            at(MatchState::Unavailable, 0, live).sync_text(),
            "Not in your libraries"
        );
        assert_eq!(
            at(MatchState::Unavailable, 0, Connection::Reconnecting).sync_text(),
            "Reconnecting"
        );
        assert!(at(MatchState::Mismatched, 0, live).offset_adjustable());
        assert!(!at(MatchState::Matched, 0, live).offset_adjustable());
        assert!(at(MatchState::Matched, 200, live).offset_adjustable());
        assert!(!at(MatchState::Searching, 200, live).offset_adjustable());
    }

    #[test]
    fn notices_name_the_peer_and_skip_my_own_actions() {
        let action = |kind: &str, by_me| Event::Action {
            kind: kind.into(),
            by: Some("p2".into()),
            by_me,
        };
        let s = PartyState::default();
        assert_eq!(
            action("pause", false).text(&s).as_deref(),
            Some("Someone paused")
        );
        assert_eq!(action("pause", true).text(&s), None);
        assert_eq!(action("wave", false).text(&s), None);
        assert_eq!(
            Event::Ended {
                reason: EndReason::Left,
                error: None
            }
            .text(&s),
            None
        );
    }
}
