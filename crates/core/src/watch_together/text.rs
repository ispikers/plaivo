use super::party::{EndReason, Event, PartyError, PartyState};
use super::protocol::{Match, MatchState, PlayerState};

impl PartyError {
    pub fn text(&self) -> Option<&'static str> {
        Some(match self.code.as_str() {
            "cancelled" => return None,
            "invalid_code" => "That isn't a valid room code.",
            "room_not_found" => "No room with that code.",
            "room_full" => "That room is full.",
            "room_locked" | "locked" => "That room is locked.",
            "relay_full" => "The Watch Together servers are busy. Try again soon.",
            "unreachable" => "Couldn't reach the Watch Together server.",
            "forbidden" | "not_host" => "Only the host can do that in this room.",
            "not_in_room" => "Join a room first.",
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
        ] {
            let text = PartyError::local(code).text();
            assert!(text.is_some() && text != fallback, "{code}");
        }
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
