use crate::types::{new_player_stats, ResourceAmount};
use crate::{
    game_round::{GameRound, RoundState},
    types::ReputationAmount,
};
use hdk::prelude::*;
use std::{collections::HashMap, time::SystemTime};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SessionState {
    InProgress,
    Lost { last_round: EntryHash },
    // TODO: when validating things, check that last game round is finished to verify
    // that session itself is finished
    Finished { last_round: EntryHash },
}

#[derive(Clone, Debug, Serialize, Deserialize, Copy)]
pub struct GameParams {
    pub regeneration_factor: f32,
    pub start_amount: ResourceAmount,
    pub num_rounds: u32,
    pub resource_coef: u32,
    pub reputation_coef: u32,
}

#[hdk_entry(id = "game_session", visibility = "public")]
#[derive(Clone)]
pub struct GameSession {
    pub owner: AgentPubKey, // who started the game
    // pub created_at: Timestamp,     // when the game was started
    pub status: SessionState,      // how the game is going
    pub game_params: GameParams,   // what specific game are we playing
    pub players: Vec<AgentPubKey>, // who is playing
}

#[derive(Clone, Debug, Serialize, Deserialize, SerializedBytes)]
pub struct GameSessionInput {
    pub game_params: GameParams,
    pub players: Vec<AgentPubKey>,
}

#[derive(Debug, Serialize, Deserialize, SerializedBytes)]
pub struct SignalPayload {
    pub game_session: GameSession,
    pub game_session_entry_hash: EntryHash,
    pub previous_round: GameRound,
    pub previous_round_entry_hash: EntryHash,
}

#[hdk_entry(id = "game_scores", visibility = "public")]
#[derive(Clone)]
pub struct GameScores {
    pub game_session: GameSession,
    pub game_session_entry_hash: EntryHash,
    //TODO add the actual results :-)
}

/*
validation rules:

- make sure session is created only when invites are answered and there's at least one accepted
    - TODO: add addresses of accepted invites into game session, later

*/

impl GameSession {
    // called in different contexts:
    // if validation: if round isn't available, validation sin't finished
    // if session state update: round is available
    pub fn update_state(&self, _game_round: GameRound) {
        // this is called every time after GameRound is created

        // if round is lost <= 0:
        //  game session is lost
        // elif number round == num_rounds:
        //  game session is finished
        // else:
        //  game session is in progress
    }
}

//external function that can be called from UI/test, until invitation zoom is added.
#[hdk_extern]
pub fn start_new_session(player_list: Vec<AgentPubKey>) -> ExternResult<HeaderHash> {
    let input = GameSessionInput {
        game_params: GameParams {
            regeneration_factor: 1.1,
            start_amount: 100,
            num_rounds: 3,
            resource_coef: 3,
            reputation_coef: 2,
        },
        players: player_list,
    };
    new_session(input)
}

pub fn new_session(input: GameSessionInput) -> ExternResult<HeaderHash> {
    // NOTE: we create a new session already having invites answered by everyone invited
    // and invite zome handles invite process before this fn call
    let agent_info: AgentInfo = agent_info()?; // agent that starts new game

    // todo:
    // get timestamp

    let latest_pubkey = agent_info.agent_latest_pubkey;
    // create entry for game session
    let gs = GameSession {
        owner: agent_info.agent_initial_pubkey.clone(),
        status: SessionState::InProgress,
        game_params: input.game_params,
        players: input.players.clone(),
    };
    create_entry(&gs)?;
    let entry_hash_game_session = hash_entry(&gs)?;

    // make link from every players agent address to game session entry
    // tixel: this is not needed I think, implicit links are in game session
    // might only be needed if remote_signal for some reason would proof to be unreliable
    // e-nastasia: I think we'll need it to implement "list all games I've created"
    // functionality for any user.
    // create_link(agent_info.agent_initial_pubkey.clone().into(),
    // entry_hash_game_session.clone(), LinkTag::new("game_sessions"))?;

    // create game round results for round 0
    // this is starting point for all the game moves of round 1 to reference (implicit link)
    let no_moves: Vec<EntryHash> = vec![];

    let round_zero = GameRound::new(
        0,
        entry_hash_game_session.clone(),
        RoundState::new(
            // NOTE(e-nastasia): we don't have to do clone() for the start_amount
            // because it's one of the primitive types that implement the Copy trait
            // so it's value will be copied instead of being moved
            gs.game_params.start_amount,
            // TODO: this fn call breaks everything as we don't have serialization implemented
            //  for the PlayerStats container type
            //new_player_stats(input.players.clone()),
            give_all_players_full_stats(gs.game_params, input.players.clone()),
        ),
        no_moves,
    );
    let header_hash_round_zero = create_entry(&round_zero)?;
    let entry_hash_round_zero = hash_entry(&round_zero)?;

    // use remote signals from RSM to send a real-time notif to invited players
    //  ! using remote signal to ping other holochain backends, instead of emit_signal
    //  that would talk with the UI
    // NOTE: we're sending signals to notify that a new round has started and
    // that players need to make their moves
    // WARNING: remote_signal is fire and forget, no error if it fails, might be a weak point if this were production happ
    let signal_payload = SignalPayload {
        // tixel: not sure if we need the full objects or only the hashes or both. The tests will tell...
        game_session: gs.clone(),
        game_session_entry_hash: entry_hash_game_session,
        previous_round: round_zero,
        previous_round_entry_hash: entry_hash_round_zero,
    };
    let signal = ExternIO::encode(GameSignal::StartNextRound(signal_payload))?;
    remote_signal(signal, input.players.clone())?;
    tracing::debug!("sending signal to {:?}", input.players.clone());

    // // todo: get timestamp as systime
    // create_entry(&calendar_event)?;

    Ok(header_hash_round_zero)
}

fn give_all_players_full_stats(
    _gp: GameParams,
    _p: Vec<AgentPubKey>,
) -> HashMap<AgentPubKey, (ResourceAmount, ReputationAmount)> {
    // TODO add map that gives all players full starting stats <AgentPubKey, (ResourceAmount, ReputationAmount)>,
    HashMap::new()
}

#[derive(Serialize, Deserialize, SerializedBytes, Debug)]
#[serde(tag = "signal_name", content = "signal_payload")]
pub enum GameSignal {
    StartNextRound(SignalPayload),
    GameOver(GameScores),
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixt::prelude::*;
    use hdk::prelude::*;
    use std::vec;

    #[test]
    fn test_new_session() {
        let mut mock_hdk = hdk::prelude::MockHdkT::new();
        let game_params = GameParams {
            regeneration_factor: 1.1,
            start_amount: 100,
            num_rounds: 3,
            resource_coef: 3,
            reputation_coef: 2,
        };

        // mock agent info
        let agent_pubkey = fixt!(AgentPubKey);
        let agent_info = AgentInfo::new(agent_pubkey.clone(), agent_pubkey.clone());
        let agent2_pubkey = fixt!(AgentPubKey);

        mock_hdk
            .expect_agent_info()
            .times(1)
            .return_once(move |_| Ok(agent_info));

        // mock create entry
        let headerhash = fixt!(HeaderHash);

        let entryhash = fixt!(EntryHash);
        let closure_header_hash = headerhash.clone();
        mock_hdk
            .expect_create()
            .with(hdk::prelude::mockall::predicate::eq(
                EntryWithDefId::try_from(GameSession {
                    owner: agent_pubkey.clone(),
                    status: SessionState::InProgress,
                    game_params: game_params.clone(),
                    players: vec![agent_pubkey.clone(), agent2_pubkey],
                })
                .unwrap(),
            ))
            .times(1)
            .return_once(move |_| Ok(closure_header_hash));

        let input = GameSessionInput {
            game_params: game_params,
            players: vec![fixt!(AgentPubKey), fixt!(AgentPubKey), fixt!(AgentPubKey)], // 3 random players
        };

        let entry_hash_game_session = fixt!(EntryHash);
        mock_hdk
            .expect_hash_entry()
            .times(1)
            .return_once(move |_| Ok(entry_hash_game_session));

        mock_hdk
            .expect_remote_signal()
            .times(1)
            .return_once(move |_| Ok(()));

        let header_hash_link = fixt!(HeaderHash);
        mock_hdk
            .expect_create_link()
            .times(1)
            .return_once(move |_| Ok(header_hash_link));

        hdk::prelude::set_hdk(mock_hdk);
        new_session(input);
    }
}
