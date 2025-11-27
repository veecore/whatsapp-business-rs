//! # WhatsApp Bulls & Cows Game Bot
//!
//! This crate contains the main application logic for a turn-based Bulls & Cows
//! game played via WhatsApp. It uses a state machine pattern to manage game rooms
//! and user interactions.
//!
//! The main components are:
//! - `App`: The main entry point that handles incoming webhooks.
//! - `Room`: Represents a single game session with a user, managed by a state machine.
//! - `RoomState`: An enum representing the current state of a game room (e.g., Start, Game, End).
//! - `GameMode`: An enum for the different types of games (e.g., HumanVsBot).

// The helpers module contains UI logic: message text, button definitions,
// and parsers for user input. This keeps the main state machine logic clean.
#[cfg(feature = "batch_server")]
mod batch_server;
mod helpers;

use dashmap::DashMap; // A thread-safe, high-performance HashMap. Perfect for storing per-user game rooms.
use std::{env, fmt::Display, ops::DerefMut, sync::Arc};

// Import necessary types from the core game logic crate.
// `call_n_said` is the Bulls & Cows engine.
use call_n_said::Guess as Call;
use call_n_said::Guess as Password;
use call_n_said::calls::Calls; // The solver/game state for a single player.

// Import local helper types.
use helpers::GameMode as GameModeRef; // A simple, copyable enum reference to GameMode.
use helpers::*; // Import all helper functions (parsing, sending messages).

// Import required components from the WhatsApp Business API crate.
use whatsapp_business_rs::{
    Client, Server, WebhookHandler,
    app::SubscriptionField,
    server::{EventContext, IncomingMessage},
};

/// Holds all application configuration loaded from environment variables.
#[derive(Debug)]
pub struct Config {
    access_token: String,
    port: String,
    app_id: String,
    app_secret: String,
    server_verify_token: String,
    server_url: String,
    super_user: String,
}

impl Config {
    /// Loads configuration from environment variables.
    /// Expects a .env file or environment variables to be set.
    fn load() -> Result<Config, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok(); // Load .env file if it exists, ignore if not.
        Ok(Config {
            access_token: env::var("WHATSAPP_ACCESS_TOKEN")
                .map_err(|_| "Please set the `WHATSAPP_ACCESS_TOKEN` env-var")?,
            port: env::var("PORT").unwrap_or_else(|_| "8080".to_string()),
            app_id: env::var("APP_ID").map_err(|_| "Please set the `APP_ID` env-var")?,
            app_secret: env::var("APP_SECRET")
                .map_err(|_| "Please set the `APP_SECRET` env-var")?,
            server_verify_token: env::var("SERVER_VERIFY_TOKEN")
                .map_err(|_| "Please set the `SERVER_VERIFY_TOKEN` env-var")?,
            server_url: env::var("SERVER_URL")
                .map_err(|_| "Please set the `SERVER_URL` env-var")?,
            super_user: env::var("SUPER_USER")
                .map_err(|_| "Please set the `SUPER_USER` env-var")?,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Loading config....");
    let config = Config::load()?;

    // 1. Create the client, which is used to *send* messages.
    let client = Client::new(config.access_token.clone()).await.unwrap();

    {
        let token = client.app(config.app_id).update_token(config.access_token, config.app_secret).await?;
        println!("{token}")
    }

todo!();
    // 2. Build the webhook server. This is what receives messages from Meta.
    let server = Server::builder()
        .endpoint(format!("127.0.0.1:{}", config.port).parse().unwrap())
        // Use the App Secret to verify payloads, ensuring they come from Meta.
        // We do this to prevent impersonation of players.
        .verify_payload(config.app_secret.clone())
        .build();

    // 3. Create our App instance, which holds the game state.
    #[cfg(feature = "batch_server")]
    let mut app = App::new(client.clone());

    #[cfg(not(feature = "batch_server"))]
    let mut app = App::new();

    #[cfg(feature = "sudo")]
    {
        app.super_user = config.super_user.into();
    }

    println!("Server starting....");
    // 4. Start the server.
    server
        .serve(app, client.clone()) // Pass our App (as WebhookHandler) and the Client.
        // 5. Register the webhook with Meta. This is a one-time setup.
        .register_webhook(
            client
                .app(config.app_id.clone())
                .configure_webhook((config.server_verify_token, config.server_url))
                // We only care about `messages` for this bot.
                .events([SubscriptionField::Messages].into())
                .with_auth((config.app_id, config.app_secret)),
        )
        .await?;

    Ok(())
}

/// The main application struct.
/// It holds the state for all ongoing games in a thread-safe manner.
#[cfg_attr(not(feature = "batch_server"), derive(Default))]
pub struct App {
    /// `Rooms` is a thread-safe map (using DashMap) that stores a `Room` (game state)
    /// for each user, keyed by their phone ID.    
    rooms: Rooms,
    #[cfg(feature = "batch_server")]
    batch_client: batch_server::BatchClient,
    #[cfg(feature = "sudo")]
    super_user: Box<str>,
}

impl App {
    /// Creates a new App instance.
    #[cfg(not(feature = "batch_server"))]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new App instance.
    #[cfg(feature = "batch_server")]
    pub fn new(client: Client) -> Self {
        let (batch_client, batch_server) = batch_server::new_batch_server();
        batch_server.spawn(client);
        Self {
            rooms: Rooms::default(),
            batch_client,
            #[cfg(feature = "sudo")]
            // God forgive me
            super_user: "".into() 
        }
    }

    /// Retrieves the game room for a given user, creating a new one if it doesn't exist.
    /// This is the core of our per-user state management.
    #[inline]
    fn get_or_start_room<'a>(&'a self, room_id: RoomId) -> impl DerefMut<Target = Room> + 'a {
        // `DashMap::entry().or_default()` is an atomic operation.
        // It gets a mutable reference to the user's room, or inserts a new
        // `Room::default()` if one doesn't exist, and returns the reference.
        // This prevents race conditions and is very efficient.
        self.rooms.0.entry(room_id.0.into()).or_default()
    }

    #[inline]
    #[cfg(feature = "batch_server")]
    fn start_chat(&self, msg: IncomingMessage) -> Chat {
        Chat::new(msg, self.batch_client.clone())
    }

    #[cfg(feature = "sudo")]
    #[inline(never)]
    async fn handle_sudo(&self, msg: &Chat, action: SudoAction) {
        match action {
            SudoAction::Help => SudoAction::send_help(&msg).await,
            SudoAction::CountRooms { filter } => {
                let count = match filter {
                    CountRoomFilter::InGame => self
                        .rooms
                        .0
                        .iter()
                        .filter(|room| matches!(room.state, RoomState::Game(..)))
                        .count(),
                    CountRoomFilter::JustStarting => self
                        .rooms
                        .0
                        .iter()
                        .filter(|room| matches!(room.state, RoomState::Start))
                        .count(),
                    CountRoomFilter::Ended => self
                        .rooms
                        .0
                        .iter()
                        .filter(|room| matches!(room.state, RoomState::End(..)))
                        .count(),
                    CountRoomFilter::All => self.rooms.0.len(),
                };
                SudoAction::send_count(msg, count).await
            }
        }
    }
}

/// A unique identifier for a game room, typically the user's phone ID.
/// This is a "newtype" wrapper for `&str` to provide type safety.
#[derive(Clone, Copy)]
pub struct RoomId<'a>(pub &'a str);

/// A thread-safe collection of all active game rooms.
/// Wraps the DashMap in an `Arc` so it can be safely shared across threads.
#[derive(Default)]
pub struct Rooms(Arc<DashMap<String, Room>>);

/// Implementation of the `WebhookHandler` trait to process incoming messages.
/// This is the main entry point for all webhooks from Meta.
impl WebhookHandler for App {
    /// This function is called every time a user sends a message to the bot.
    async fn handle_message(&self, _ctx: EventContext, msg: IncomingMessage) {
        // Spawn a background task to set the user's status to "typing...".
        // This is good UX, but we don't wait for it.
        tokio::spawn(msg.set_replying().into_future());

        #[cfg(debug_assertions)]
        println!("Got a message: {msg}\n");

        // Use the sender's phone ID as a unique identifier for their game room.
        let room_id = RoomId(&msg.sender.phone_id);

        // Get or create the user's specific game room.
        // `room` is a mutable guard to the `Room` value in the DashMap.
        let mut room = self.get_or_start_room(room_id);

        #[cfg(feature = "batch_server")]
        let msg = self.start_chat(msg);

        #[cfg(feature = "sudo")]
        if *msg.sender.phone_id == *self.super_user {
            if let Some(sudo) = SudoAction::parse(&msg) {
                drop(room); // to avoid deadlock
                self.handle_sudo(&msg, sudo).await;
                return;
            }
            // fallthrough
        }

        // --- IMGUI-style Update ---
        // Delegate all logic to the room's `update` method.
        // This is the core of the "immediate mode" pattern: we call one `update`
        // function with the new input (the message), and let the state
        // machine handle everything else.
        room.update(msg).await;
    }

    /// This function handles message status updates (e.g., "sent", "delivered", "read").
    /// We did not subscribe to this(MessageStatusUpdate) in this example but is good for debugging.
    async fn handle_message_update(
        &self,
        _ctx: EventContext,
        update: whatsapp_business_rs::server::MessageUpdate,
    ) {
        // We don't need to do anything for successful updates,
        // but we'll log if a message fails to send.
        if update.is_sent()
            || update.is_accepted()
            || update.is_delivered()
            || update.is_read()
            || update.is_deleted()
        {
            // Happy path, do nothing.
        } else {
            // Log undelivered messages.
            println!("Undelivered Message: {update:#?}")
        }
    }
}

/// Represents a single game session and its current state.
#[derive(Default)]
pub struct Room {
    /// This is the state machine. `RoomState` is an enum that holds
    /// the data for the *current* state (e.g., `Start`, `Game`, `End`).    
    state: RoomState,
}

impl Room {
    /// Creates a new room, defaulting to the Start state.
    pub fn new() -> Self {
        Self::default()
    }

    /// The main update loop for a room.
    /// This is the core of the "Immediate Mode" (IMGUI) pattern.
    ///
    /// 1. We `match` on the *current* state (`self.state`).
    /// 2. We call the `update` function for that *specific* state.
    /// 3. That `update` function handles the message and returns an `Option<RoomState>`.
    ///    - `None`: The state does not change.
    ///    - `Some(new_state)`: A state transition is requested.
    /// 4. If we get `Some(new_state)`, we update `self.state` to transition.
    ///
    /// This is like a UI "render loop" that runs for every "input event" (message).
    #[inline]
    async fn update(&mut self, msg: Chat) {
        // Delegate to the current state's update logic.
        let new_state = match &mut self.state {
            RoomState::Start => Self::update_start(msg).await,
            RoomState::Game(game) => game.update(msg).await, // Delegate to the GameMode's update
            RoomState::End(end) => end.update(msg).await,    // Delegate to the End state's update
        };

        // If the update function returned a new state, perform the transition.
        if let Some(state) = new_state {
            self.state = state;
        }
    }

    /// Creates a new room in the `Start` state and sends the welcome message.
    /// This is a helper for transitioning *back* to the main menu.
    #[inline]
    async fn start(msg: &Chat) -> RoomState {
        StartAction::send_hello(msg).await; // Send the main menu message.
        RoomState::Start // Return the new state.
    }

    /// Handles user input when the room is in the `Start` state.
    /// Returns `Some(RoomState)` to transition, or `None` to stay in `Start`.    
    #[inline]
    async fn update_start(msg: Chat) -> Option<RoomState> {
        // `StartAction::parse` (from `helpers.rs`) figures out what the user wants.
        match StartAction::parse(&msg) {
            // User wants to start a game.
            StartAction::Start { mode } => {
                // `mode.start()` creates the new game state (e.g., `RoomState::Game(...)`).
                // We return `Some(...)` to signal a state transition.
                Some(mode.start(&msg).await)
            }
            // User said "Hi" or needs help.
            StartAction::Hi => {
                StartAction::send_hello(&msg).await; // Re-send the welcome/help message.
                None // Stay in the `Start` state.
            }
            StartAction::LearnMore => {
                StartAction::send_learn_more(&msg).await; // Send game rules.
                None // Stay in the `Start` state.
            }
            // User sent an invalid command.
            StartAction::Invalid { err } => {
                StartAction::reply_invalid(&msg, err).await; // Send an error message.
                None // Stay in the `Start` state.
            }
        }
    }
}

/// Represents the possible states a `Room` can be in.
/// This is the top-level state machine.
#[derive(Default, Debug)]
#[must_use = "Remember to signal update"] // Linter check
pub enum RoomState {
    #[default] // The default state for a new `Room`
    Start,
    End(End),
    Game(GameMode), // This state contains the *nested* game state machine.
}

/// The state for a completed game.
#[derive(Debug)]
pub struct End {
    last: GameModeRef,       // Remembers the last game mode for "Play Again".
    win_msg: Option<String>, // Stores a shareable win message if the human won.
}

impl End {
    /// Transitions to the `End` state from a completed game.
    #[inline]
    async fn start(msg: &Chat, game: GameModeRef) -> RoomState {
        // The specific win/loss message has already been sent by the game mode.
        // This function sends the follow-up actions like "Play Again" or "Main Menu".
        EndAction::send_end_prompt(msg, game).await;
        RoomState::End(Self {
            last: game,
            win_msg: None, // No win message by default (e.g., if bot won).
        })
    }

    /// A special transition for when the *human* wins, storing their win message.
    #[inline]
    async fn with_win_msg(msg: &Chat, game: GameModeRef, win_msg: impl Display) -> RoomState {
        EndAction::send_end_prompt_suggest_win(msg, game).await; // Send "Play Again" + "Share Win"
        RoomState::End(Self {
            last: game,
            win_msg: Some(win_msg.to_string()),
        })
    }

    /// Handles user input when in the `End` state (e.g., play again, main menu).
    #[inline]
    async fn update(&mut self, msg: Chat) -> Option<RoomState> {
        match EndAction::parse(&msg) {
            // User wants to go to the main menu.
            EndAction::MainMenu => {
                // `Room::start` sends the welcome message and returns `RoomState::Start`.
                // We return `Some(...)` to transition.
                Some(Room::start(&msg).await)
            }
            // User wants to play the same game mode again.
            EndAction::PlayAgain => {
                // `self.last.start` creates a new game state of the same type.
                // We return `Some(...)` to transition.
                Some(self.last.start(&msg).await)
            }
            // User wants to share their win.
            EndAction::ShareWin => {
                if let Some(win_msg) = &self.win_msg {
                    EndAction::send_share_win(&msg, win_msg).await;
                } else {
                    EndAction::send_share_win_no_win(&msg).await
                }
                None // Stay in the `End` state.
            }
            EndAction::Invalid { err } => {
                EndAction::reply_invalid(&msg, err).await;
                None // Stay in the `End` state.
            }
        }
    }
}

/// Represents the different game modes available.
/// This is a *nested* state machine, contained within `RoomState::Game`.
#[derive(Debug)]
pub enum GameMode {
    BvB(BotVsBot),
    HvB(HumanVsBot),
    Assist(AssistMode),
}

impl GameModeRef {
    /// Factory method to create the starting `RoomState` for a new game.
    #[inline]
    async fn start(self, msg: &Chat) -> RoomState {
        match self {
            GameModeRef::BvB => BotVsBot::start(msg).await,
            GameModeRef::HvB => HumanVsBot::start(msg).await,
            GameModeRef::Assist => AssistMode::start(msg).await,
        }
    }
}

impl GameMode {
    /// Main update loop for *any* active game mode.
    /// This handles generic commands (like "restart") *before* delegating
    /// to the specific game mode's logic.    
    #[inline]
    async fn update(&mut self, msg: Chat) -> Option<RoomState> {
        // First, try to parse a generic command applicable to any game mode.
        if let Some(command) = GameRoomAction::parse(&msg) {
            match command {
                // User wants to restart the *current* game type.
                GameRoomAction::Restart => Some(self.restart(&msg).await),
                // User wants help for the *current* game type.
                GameRoomAction::Help => {
                    self.help(&msg).await;
                    None // Stay in the same game state.
                }
                // User wants to go back to the main menu.
                GameRoomAction::MainMenu => Some(Room::start(&msg).await),
            }
        } else {
            // If it's not a generic command, it must be game-specific input
            // (e.g., a guess like "1234" or a grade like "1b 2c").
            // Delegate to the specific game mode's update logic.
            match self {
                Self::BvB(bot_vs_bot) => bot_vs_bot.update(msg).await,
                Self::HvB(human_vs_bot) => human_vs_bot.update(msg).await,
                Self::Assist(assist_mode) => assist_mode.update(msg).await,
            }
        }
    }

    /// Restarts the current game mode.
    #[inline]
    async fn restart(&mut self, msg: &Chat) -> RoomState {
        match self {
            GameMode::BvB(_) => BotVsBot::start(msg).await,
            GameMode::HvB(_) => HumanVsBot::start(msg).await,
            GameMode::Assist(_) => AssistMode::start(msg).await,
        }
    }

    /// Sends a help message specific to the current game mode.
    #[inline]
    async fn help(&mut self, msg: &Chat) {
        match self {
            GameMode::BvB(_) => BotVsBot::help(msg).await,
            GameMode::HvB(_) => HumanVsBot::help(msg).await,
            GameMode::Assist(_) => AssistMode::help(msg).await,
        }
    }
}

/// Game state for a Bot vs. Bot match.
#[derive(Debug)]
pub struct BotVsBot {
    bot_a: (Calls, Password), // Bot A's solver state and its secret password
    bot_b: (Calls, Password), // Bot B's solver state and its secret password
}

impl BotVsBot {
    /// Starts a new BvB game.
    #[inline]
    async fn start(msg: &Chat) -> RoomState {
        let mut me = Self {
            bot_a: (Calls::default(), Password::random()),
            bot_b: (Calls::default(), Password::random()),
        };
        // BvB is fully automatic. We immediately advance to the first turn
        // and send the result.
        if let Some(new_state) = me.advance(msg).await {
            // The game might end on the first turn (a lucky guess).
            new_state
        } else {
            // Otherwise, return the new game state.
            RoomState::Game(GameMode::BvB(me))
        }
    }

    #[inline]
    async fn help(msg: &Chat) {
        BvbAction::send_help(msg).await
    }

    /// Handles user input *during* a BvB game.
    /// The only valid input is "Next" or a generic command.
    #[inline]
    async fn update(&mut self, msg: Chat) -> Option<RoomState> {
        match BvbAction::parse(&msg) {
            // User clicked "Next" to see the next turn.
            BvbAction::Next => self.advance(&msg).await,
            BvbAction::Invalid { err } => {
                BvbAction::reply_invalid(&msg, err).await;
                None // Stay in this state.
            }
        }
    }

    /// Advances the game by one turn for each bot.
    /// Returns `Some(RoomState::End)` if the game is over.
    #[inline]
    async fn advance(&mut self, msg: &Chat) -> Option<RoomState> {
        // --- Bot A's turn ---
        let a_call = self
            .bot_a
            .0
            .guess()
            .expect("Bot A cannot guess, game should have ended");
        let a_call_result = self.bot_b.1.score(a_call); // Score A's guess vs B's password
        self.bot_a.0.said(a_call, a_call_result); // Update A's solver
        let response = BvbAction::reply(msg).set_a_call(a_call, a_call_result);

        // Check for win.
        if a_call_result.is_win() {
            response.send_a_wins(self.bot_a.1).await;
            return Some(self.end(msg).await); // Transition to End state
        }

        // --- Bot B's turn ---
        let b_call = self
            .bot_b
            .0
            .guess()
            .expect("Bot B cannot guess, game should have ended");
        let b_call_result = self.bot_a.1.score(b_call); // Score B's guess vs A's password
        self.bot_b.0.said(b_call, b_call_result); // Update B's solver
        let response = response.set_b_call(b_call, b_call_result);

        // Check for win.
        if b_call_result.is_win() {
            response.send_b_wins(self.bot_b.1).await;
            return Some(self.end(msg).await); // Transition to End state
        }

        // If no winner, send the turn summary and stay in this state.
        response.send().await;
        None
    }

    /// Helper to transition to the `End` state.
    #[inline]
    async fn end(&self, msg: &Chat) -> RoomState {
        End::start(msg, GameModeRef::BvB).await
    }
}

/// Game state for a Human vs. Bot match.
#[derive(Debug)]
pub struct HumanVsBot {
    bot: (Calls, Password), // Bot's solver and its secret password
    /// This is key sub-state: if the bot has made a guess, we store it here
    /// and wait for the human to provide a grade (e.g., "1b 2c").
    last_bot_call: Option<Call>,
    /// Optional: If the human provides their password (e.g., "set 1234"),
    /// we store it here to auto-grade the bot's guesses.
    human_password: Option<Password>,
}

impl HumanVsBot {
    /// Starts a new HvB game.
    #[inline]
    async fn start(msg: &Chat) -> RoomState {
        let me = Self {
            bot: (Calls::new(), Password::random()),
            last_bot_call: None, // Human goes first.
            human_password: None,
        };
        HvbAction::send_welcome(msg).await;
        RoomState::Game(GameMode::HvB(me))
    }

    #[inline]
    async fn help(msg: &Chat) {
        HvbAction::send_help(msg).await;
    }

    /// The core logic loop for the Human vs Bot game.
    #[inline]
    async fn update(&mut self, msg: Chat) -> Option<RoomState> {
        // `HvbAction::parse` is smart. It can detect:
        // - "1234" (a Call)
        // - "1b 2c" (a Grade)
        // - "1b 2c 1234" (Both)
        // - "set 1234" (SetPassword)
        match HvbAction::parse(&msg) {
            HvbAction::Grade(grade) => self.handle_grade(&msg, grade).await,
            HvbAction::Call(call) => self.handle_call(&msg, call).await,
            HvbAction::Both { grade, call } => self.handle_both(&msg, grade, call).await,
            HvbAction::SetPassword(password) => {
                if let Some(former) = self.human_password {
                    HvbAction::send_user_password_already_saved(&msg, former, password).await;
                } else {
                    self.human_password = Some(password);
                    HvbAction::send_user_password_saved(&msg, password).await;
                }
                None // Stay in this state.
            }
            HvbAction::Invalid { err } => {
                HvbAction::reply_invalid(&msg, err).await;
                None // Stay in this state.
            }
        }
    }

    /// Handles the case where the user *only* provides a grade for the bot's last call.
    #[inline]
    async fn handle_grade(&mut self, msg: &Chat, grade: GradeAction) -> Option<RoomState> {
        let bot_grade = grade.grade;

        // --- 1. Process the Grade for the bot's last call ---
        // `try_settle_grade` handles validating the grade and checking for a bot win.
        if let Err(err) = self.try_settle_grade(msg, grade).await {
            return err; // `err` might be `Some(RoomState::End)` if the bot won.
        }

        // --- 2. Prompt user to make their call ---
        // If the bot didn't win, it's now the human's turn to make a call.
        HvbAction::send_promt_user_to_call(msg, bot_grade).await;
        None // Stay in this state.
    }

    /// Tries to apply a user-provided grade to the bot's last call.
    /// This is a fallible function.
    /// `Err(None)` means a recoverable error occurred (e.g., "it's not my turn").
    /// `Err(Some(RoomState))` means a game-ending event occurred (bot won).
    /// `Ok(())` means the grade was applied successfully.    
    #[inline]
    async fn try_settle_grade(
        &mut self,
        msg: &Chat,
        grade: GradeAction,
    ) -> Result<(), Option<RoomState>> {
        // If the user set a password, they shouldn't be grading manually.
        if self.human_password.is_some() {
            HvbAction::send_user_grade_not_needed(msg).await;
            return Err(None);
        }

        // Check if we are *waiting* for a grade.
        let Some(last_call) = self.last_bot_call.take() else {
            // We weren't waiting for a grade. It's the user's turn to call.
            HvbAction::send_is_not_bot_turn(msg).await;
            return Err(None);
        };

        // For now, we don't support grading a call other than the most recent one.
        if let Some(target) = grade.target
            && target != last_call
        {
            HvbAction::send_target_unsupported(msg, last_call).await;
            self.last_bot_call = Some(last_call); // Put it back since it wasn't graded.
            return Err(None);
        }

        let grade = grade.grade;
        // Update the bot's solver with the new information.
        self.bot.0.said(last_call, grade);

        // Check if the bot won.
        if grade.is_win() {
            HvbAction::send_bot_wins(msg).await;
            // Return `Err(Some(EndState))` to signal a state transition.
            return Err(Some(self.end(msg).await));
        }

        // Grade applied successfully.
        Ok(())
    }

    /// Handles the case where the user *only* makes a new call.
    #[inline]
    async fn handle_call(&mut self, msg: &Chat, call: Call) -> Option<RoomState> {
        // --- 1. Check if it's the user's turn to make a call ---
        if let Some(last_call) = self.last_bot_call {
            // It's not the user's turn. They must grade the bot's last call first.
            HvbAction::send_last_call_not_graded(msg, last_call).await;
            return None; // Stay in this state.
        }

        // --- 2. Process the User's Call ---
        let user_score = self.bot.1.score(call); // Score user's call vs bot's password.
        if user_score.is_win() {
            // Human wins!
            HvbAction::send_human_wins(msg, self.bot.0.history().len(), self.bot.1).await;
            // Transition to End state with a shareable win message.
            return Some(self.end_human_wins(msg).await);
        }

        // --- 3. Make the Bot's next move ---
        let Some(next_bot_call) = self.bot.0.guess() else {
            // The bot has no more possible guesses. This means the user
            // gave contradictory grades.
            HvbAction::send_discrepancy(msg).await;
            return Some(self.end(msg).await); // End the game.
        };

        // --- 4. Reply with the user's score and the bot's new call ---
        if let Some(human_password) = self.human_password {
            // --- Auto-Grading Path ---
            let bot_grade = human_password.score(next_bot_call); // Auto-grade the bot
            self.bot.0.said(next_bot_call, bot_grade); // Update bot's solver

            // `last_bot_call` remains `None` because the turn was auto-graded.
            // The bot is ready for the user's next call.
            self.last_bot_call = None;

            // Send a single message with both results.
            HvbAction::send_grade_n_call_n_grade_myself(
                msg,
                GradeAction {
                    grade: user_score,
                    target: Some(call),
                },
                GradeAction {
                    grade: bot_grade,
                    target: Some(next_bot_call),
                },
            )
            .await;

            // Check if the bot won.
            if bot_grade.is_win() {
                HvbAction::send_bot_wins(msg).await;
                return Some(self.end(msg).await); // Transition to End.
            }
        } else {
            // --- Manual Grading Path ---
            // Store the bot's call, so we wait for a grade.
            self.last_bot_call = Some(next_bot_call);
            // Send the user's score and the bot's new call, asking for a grade.
            HvbAction::send_grade_n_call(
                msg,
                GradeAction {
                    grade: user_score,
                    target: Some(call),
                },
                next_bot_call,
            )
            .await;
        }

        None // Stay in this state.
    }

    /// Handles the case where the user provides a grade *and* makes a new call
    /// in the same message (e.g., "1b 0c 5678").
    #[inline]
    async fn handle_both(
        &mut self,
        msg: &Chat,
        grade: GradeAction,
        call: Call,
    ) -> Option<RoomState> {
        // --- 1. Process the Grade for the bot's previous call ---
        if let Err(err) = self.try_settle_grade(msg, grade).await {
            // If grading fails or the bot wins, stop here.
            return err;
        }

        // --- 2. Handle the User's new Call ---
        // If grading was successful, proceed to handle the user's call.
        // This will score their call, make the bot's next move, and reply.
        self.handle_call(msg, call).await
    }

    /// Helper to transition to the `End` state (bot win or discrepancy).
    #[inline]
    async fn end(&self, msg: &Chat) -> RoomState {
        End::start(msg, GameModeRef::HvB).await
    }

    /// Helper to transition to the `End` state (human win).
    #[inline]
    async fn end_human_wins(&self, msg: &Chat) -> RoomState {
        // Create a shareable win message.
        End::with_win_msg(
            msg,
            GameModeRef::HvB,
            HvbAction::get_share_win_message(msg, self.bot.0.history().len()),
        )
        .await
    }
}

/// Game state for Assist Mode. The bot just suggests guesses.
#[derive(Debug)]
pub struct AssistMode {
    assists: Calls,  // The bot's solver state.
    last_call: Call, // The last guess the bot suggested.
}

impl AssistMode {
    /// Starts a new Assist Mode game.
    #[inline]
    async fn start(msg: &Chat) -> RoomState {
        let mut assists = Calls::new();
        // The first guess is always a good starting point (e.g., "1234" or "1203").
        let first_call = assists
            .guess()
            .expect("First call should always be available");
        let me = Self {
            last_call: first_call,
            assists,
        };
        // Send the welcome message with the first suggestion.
        AssistAction::send_welcome(msg, first_call).await;
        RoomState::Game(GameMode::Assist(me))
    }

    #[inline]
    async fn help(msg: &Chat) {
        AssistAction::send_help(msg).await;
    }

    /// Handles user input in Assist Mode.
    /// The only expected input is a grade (e.g., "1b 2c").
    #[inline]
    async fn update(&mut self, msg: Chat) -> Option<RoomState> {
        match AssistAction::parse(&msg) {
            // User sent a grade for the bot's last suggestion.
            AssistAction::Grade(guess_result) => {
                // Update the solver with this new information.
                self.assists.said(self.last_call, guess_result);

                // Check if that was the winning move.
                if guess_result.is_win() {
                    AssistAction::send_human_wins(&msg, self.assists.history().len()).await;
                    return Some(self.end_human_wins(&msg).await); // Transition to End.
                }

                // Get the next best guess from the solver.
                let Some(next_call) = self.assists.guess() else {
                    // No possible numbers left. The user must have made a grading error.
                    AssistAction::send_discrepancy(&msg).await;
                    // There are no possible numbers left based on the user's grading.
                    return Some(self.end(&msg).await); // Transition to End.
                };

                // Send the next suggestion.
                self.last_call = next_call;
                AssistAction::reply_next(&msg, guess_result, next_call).await;
                None // Stay in this state.
            }
            // User sent something other than a grade.
            AssistAction::Invalid { err } => {
                AssistAction::reply_invalid(&msg, err).await;
                None // Stay in this state.
            }
        }
    }

    /// Helper to transition to the `End` state (discrepancy).
    #[inline]
    async fn end(&self, msg: &Chat) -> RoomState {
        End::start(msg, GameModeRef::Assist).await
    }

    /// Helper to transition to the `End` state (human win).
    #[inline]
    async fn end_human_wins(&self, msg: &Chat) -> RoomState {
        End::with_win_msg(
            msg,
            GameModeRef::Assist,
            AssistAction::get_share_win_message(msg, self.assists.history().len()),
        )
        .await
    }
}

// --- Tests ---
// The test module uses mock messages to simulate user input
// and asserts that the `Room`'s state transitions correctly.
#[cfg(test)]
mod tests {
    #[cfg(feature = "batch_server")]
    use std::sync::LazyLock;

    use super::*;
    use call_n_said::GuessResult;
    use whatsapp_business_rs::{
        Identity, Message, Timestamp,
        message::{Button, Content, Context, InteractiveContent, MessageStatus},
        server::IncomingMessage,
    };

    #[cfg(feature = "batch_server")]
    const APP: LazyLock<App> = LazyLock::new(|| {
        let client = Client::builder().build().unwrap();
        App::new(client.clone())
    });

    #[cfg(feature = "batch_server")]
    impl From<IncomingMessage> for Chat {
        fn from(value: IncomingMessage) -> Self {
            APP.start_chat(value)
        }
    }

    // A test helper to create a mock `IncomingMessage`.
    // This is complex due to the private fields in the original struct,
    // so it uses `unsafe { std::mem::transmute }` to build it.
    pub fn new_incoming_message(content: impl Into<Content>) -> IncomingMessage {
        #[allow(dead_code)]
        struct MessageLike {
            id: String,
            sender: Identity,
            recipient: Identity,
            content: Content,
            context: Context,
            timestamp: i64,
            message_status: MessageStatus,
        }

        let message: Message = unsafe {
            std::mem::transmute(MessageLike {
                id: "wamid.ID".into(),
                sender: Identity::user("1234567890"), // Mock user ID
                recipient: Identity::business("1234567890"),
                content: content.into(),
                context: Context::default(),
                timestamp: 1678886400,
                message_status: MessageStatus::Sent,
            })
        };

        #[allow(dead_code)]
        struct IncomingMessageLike {
            message: Message,
            timestamp: Option<Timestamp>,
            client: Client,
        }

        let msg = IncomingMessageLike {
            message,
            timestamp: None,
            // A mock client that can't actually send messages.
            client: Client::builder().build().unwrap(),
        };
        unsafe { std::mem::transmute(msg) }
    }

    // Helper function to create a mock IncomingMessage with text content
    pub fn mock_text_message(body: &str) -> IncomingMessage {
        new_incoming_message(body)
    }

    // Helper function to create a mock IncomingMessage with a button click
    pub fn mock_button_message(callback: &str) -> IncomingMessage {
        new_incoming_message(InteractiveContent::Click(Button::reply(callback, "Title")))
    }

    #[tokio::test]
    async fn test_initial_state_and_start_options() {
        let mut room = Room::new();
        // A new room should always start in the `Start` state.
        assert!(matches!(room.state, RoomState::Start));

        // Test "Hi" -> should remain in Start
        room.update(mock_text_message("Hi").into()).await;
        assert!(matches!(room.state, RoomState::Start), "Failed on 'Hi'");

        // Test "Learn More" -> should remain in Start
        room.update(mock_text_message("Learn More about the game").into())
            .await;
        assert!(
            matches!(room.state, RoomState::Start),
            "Failed on 'Learn More'"
        );

        // Test Invalid command -> should remain in Start
        room.update(mock_text_message("some random gibberish").into())
            .await;
        assert!(
            matches!(room.state, RoomState::Start),
            "Failed on invalid command"
        );
    }

    #[tokio::test]
    async fn test_start_to_game_transition() {
        let mut room = Room::new();

        // Test transition to HumanVsBot
        room.update(mock_button_message(START_HVB_CALLBACK).into())
            .await;
        assert!(matches!(room.state, RoomState::Game(GameMode::HvB(_))));

        // Reset and test transition to BotVsBot
        // First, go back to main menu
        room.update(mock_button_message(GAME_ROOM_MAINMENU_CALLBACK).into())
            .await;
        assert!(matches!(room.state, RoomState::Start));

        // Then start BvB
        room.update(mock_button_message(START_BVB_CALLBACK).into())
            .await;
        // BvB immediately advances, so it can
        // either be in Game or End state (if it won on the first turn).
        let is_bvb_or_end = matches!(&room.state, RoomState::Game(GameMode::BvB(_)))
            || matches!(&room.state, RoomState::End(_));
        assert!(is_bvb_or_end, "Failed to transition to BvB");

        // Reset and test transition to Assist Mode
        room.state = RoomState::Start;
        room.update(mock_button_message(START_ASSIST_CALLBACK).into())
            .await;
        assert!(matches!(room.state, RoomState::Game(GameMode::Assist(_))));
    }

    #[tokio::test]
    async fn test_hvb_game_flow_without_password() {
        let mut room = Room::new();

        // 1. Start HvB game
        room.update(mock_button_message(START_HVB_CALLBACK).into())
            .await;

        // 2. Make first call.
        room.update(mock_text_message("1234").into()).await;
        let bot_call = if let RoomState::Game(GameMode::HvB(state)) = &mut room.state {
            assert!(state.human_password.is_none());
            // After the human calls, the bot should make a call and wait for a grade.
            assert!(state.last_bot_call.is_some());
            state.last_bot_call.unwrap()
        } else {
            panic!("Wrong state: expected HvB, got {:?}", room.state);
        };

        // 3. Try to make another call without grading (should be ignored)
        room.update(mock_text_message("5678").into()).await;
        if let RoomState::Game(GameMode::HvB(state)) = &room.state {
            // State should be unchanged, bot call is still waiting for a grade
            assert_eq!(state.last_bot_call, Some(bot_call));
        } else {
            panic!("Wrong state");
        }

        // 4. Grade the bot's call
        room.update(mock_text_message("1b 2c").into()).await;
        if let RoomState::Game(GameMode::HvB(state)) = &room.state {
            // Bot's call is now graded, so `last_bot_call` should be None
            // (it's the human's turn to call).
            assert!(state.last_bot_call.is_none());
            // Check if the bot's internal state was updated
            assert_eq!(state.bot.0.history().len(), 1);
            assert_eq!(state.bot.0.history()[0].0, bot_call);
            assert_eq!(state.bot.0.history()[0].1, GuessResult::new(1, 2).unwrap());
        } else {
            panic!("Wrong state");
        }
    }

    #[tokio::test]
    async fn test_hvb_win_conditions() {
        // 1. Player wins
        let mut room = Room::new();
        // Manually set up a game where the bot's password is "1234"
        room.state = RoomState::Game(GameMode::HvB(HumanVsBot {
            bot: (Calls::new(), parse_guess("1234").unwrap()),
            last_bot_call: None,
            human_password: None,
        }));
        room.update(mock_text_message("1234").into()).await; // Winning call
        // Should transition to End
        assert!(matches!(room.state, RoomState::End(_)));
        if let RoomState::End(end_state) = room.state {
            assert_eq!(end_state.last, GameModeRef::HvB);
            // Should have a win message to share
            assert!(end_state.win_msg.is_some());
        } else {
            panic!("Expected End state");
        }

        // 2. Bot wins
        let mut room = Room::new();
        // Manually set up a game where the bot is waiting for a grade on its "1234" guess.
        room.state = RoomState::Game(GameMode::HvB(HumanVsBot {
            bot: (Calls::new(), parse_guess("5678").unwrap()),
            last_bot_call: Some(parse_guess("1234").unwrap()),
            human_password: None,
        }));
        room.update(mock_text_message("4b 0c").into()).await; // Winning grade
        // Should transition to End
        assert!(matches!(room.state, RoomState::End(_)));
        if let RoomState::End(end_state) = room.state {
            assert_eq!(end_state.last, GameModeRef::HvB);
            // Bot winning doesn't generate a shareable message
            assert!(end_state.win_msg.is_none());
        } else {
            panic!("Expected End state");
        }
    }

    #[tokio::test]
    async fn test_hvb_with_set_password() {
        let mut room = Room::new();
        room.update(mock_button_message(START_HVB_CALLBACK).into())
            .await;

        // 1. Set password
        room.update(mock_text_message("set 5678").into()).await;
        if let RoomState::Game(GameMode::HvB(state)) = &room.state {
            assert_eq!(state.human_password, Some(parse_guess("5678").unwrap()));
            assert!(state.last_bot_call.is_none());
        } else {
            panic!("Wrong state");
        }

        // 2. Make a call. Bot should grade itself and not require user input.
        room.update(mock_text_message("1234").into()).await;
        // The game should either continue (if no winner) or end (if bot won).
        if let RoomState::Game(GameMode::HvB(state)) = &room.state {
            // The bot should have made a call and immediately graded it,
            // so `last_bot_call` should still be None (ready for user's next call).
            assert!(state.last_bot_call.is_none());
            // And it should have a history of one call.
            assert_eq!(state.bot.0.history().len(), 1);
        } else {
            // Or the bot won and the game ended
            assert!(matches!(room.state, RoomState::End(_)));
        }
    }

    #[tokio::test]
    async fn test_generic_game_commands() {
        let mut room = Room::new();
        room.update(mock_button_message(START_HVB_CALLBACK).into())
            .await;

        // Make a move to ensure there's state to reset
        room.update(mock_text_message("1234").into()).await;

        // Restart should reset the game, but stay in HvB
        room.update(mock_text_message("restart").into()).await;
        assert!(matches!(&room.state, RoomState::Game(GameMode::HvB(_))));
        if let RoomState::Game(GameMode::HvB(hvb)) = &room.state {
            // Check if it's a fresh game
            assert!(hvb.last_bot_call.is_none()); // Human's turn
            assert!(hvb.bot.0.history().is_empty()); // Bot solver is reset
        } else {
            panic!("State should be a fresh HvB game");
        }

        // Main Menu should go back to Start
        room.update(mock_text_message("main menu").into()).await;
        assert!(matches!(room.state, RoomState::Start));
    }

    #[tokio::test]
    async fn test_assist_mode_flow() {
        let mut room = Room::new();
        room.update(mock_button_message(START_ASSIST_CALLBACK).into())
            .await;

        let last_call = if let RoomState::Game(GameMode::Assist(assist)) = &room.state {
            assert_eq!(assist.assists.history().len(), 0); // No history yet
            assist.last_call // Get the first suggestion
        } else {
            panic!("Should be in assist mode");
        };

        // Grade the first suggestion
        room.update(mock_text_message("1b 0c").into()).await;

        // Check that the state was updated
        if let RoomState::Game(GameMode::Assist(assist)) = &room.state {
            assert_eq!(assist.assists.history().len(), 1); // Now has 1 entry
            assert_eq!(
                assist.assists.history()[0],
                (last_call, GuessResult::new(1, 0).unwrap())
            );
            assert_ne!(assist.last_call, last_call); // A new call should be suggested
        } else {
            panic!("Should remain in assist mode");
        }

        // Test winning
        room.update(mock_text_message("4b").into()).await;
        assert!(matches!(room.state, RoomState::End(_)));
        if let RoomState::End(end_state) = room.state {
            assert_eq!(end_state.last, GameModeRef::Assist);
            assert!(end_state.win_msg.is_some());
        } else {
            panic!("Expected End state after win");
        }
    }

    #[tokio::test]
    async fn test_assist_mode_discrepancy() {
        let mut room = Room::new();
        // Manually construct a state where a discrepancy is inevitable
        let mut assists = Calls::new();
        assists.said(
            parse_guess("1234").unwrap(),
            GuessResult::new(0, 0).unwrap(),
        ); // no 1,2,3,4
        assists.said(
            parse_guess("5678").unwrap(),
            GuessResult::new(0, 0).unwrap(),
        ); // no 5,6,7,8
        // Only 9,0 are left, impossible to form a 4-digit number.
        // Next call to `assists.guess()` will return None.
        room.state = RoomState::Game(GameMode::Assist(AssistMode {
            assists,
            last_call: parse_guess("5678").unwrap(),
        }));

        // Any grade will now trigger the next `guess()` call, which will be None
        room.update(mock_text_message("0b 0c").into()).await;

        assert!(matches!(room.state, RoomState::End(_)));
        if let RoomState::End(end_state) = room.state {
            assert_eq!(end_state.last, GameModeRef::Assist);
            assert!(end_state.win_msg.is_none());
        } else {
            panic!("Expected End state after discrepancy");
        }
    }

    #[tokio::test]
    async fn test_end_state_flow() {
        // Get into an End state from winning a game
        let mut room = Room::new();
        room.state = RoomState::End(End {
            last: GameModeRef::HvB,
            win_msg: Some("You won!".to_string()),
        });

        // Test "Play Again"
        room.update(mock_button_message(PLAY_AGAIN_CALLBACK).into())
            .await;
        assert!(matches!(&room.state, RoomState::Game(GameMode::HvB(_))));
        if let RoomState::Game(GameMode::HvB(hvb)) = &room.state {
            assert!(hvb.bot.0.history().is_empty());
        } else {
            panic!("Expected a fresh HvB game");
        }

        // Get back to End state
        room.state = RoomState::End(End {
            last: GameModeRef::HvB,
            win_msg: None,
        });

        // Test "Main Menu"
        room.update(mock_button_message(END_MAINMENU_CALLBACK).into())
            .await;
        assert!(matches!(room.state, RoomState::Start));
    }

    #[tokio::test]
    async fn test_bvb_game_flow_and_win() {
        let mut room = Room::new();

        // Let's force a state where Bot B is guaranteed to win on the next turn
        let bot_a_password = parse_guess("1234").unwrap();
        let bot_b_password = parse_guess("5678").unwrap();
        let mut bot_b_calls = Calls::new();

        // Craft a history for bot_b so that its next guess is the winning one
        bot_b_calls.said(
            parse_guess("1236").unwrap(),
            bot_a_password.score(parse_guess("1236").unwrap()),
        ); // 3b, 0c
        bot_b_calls.said(
            parse_guess("7894").unwrap(),
            bot_a_password.score(parse_guess("7894").unwrap()),
        ); // 1b, 0c
        bot_b_calls.said(
            parse_guess("1504").unwrap(),
            bot_a_password.score(parse_guess("1504").unwrap()),
        ); // 2b, 0c

        room.state = RoomState::Game(GameMode::BvB(BotVsBot {
            bot_a: (Calls::new(), bot_a_password),
            bot_b: (bot_b_calls, bot_b_password),
        }));

        // Now, advance the game. Bot A will make a guess. Bot B will then guess "1234" and win.
        // The solver logic should now have more than enough info to deduce "1234"
        room.update(mock_button_message(BVB_NEXT_CALLBACK).into())
            .await;

        assert!(matches!(room.state, RoomState::End(_)));
        if let RoomState::End(end_state) = room.state {
            assert_eq!(end_state.last, GameModeRef::BvB);
        } else {
            panic!("Expected End state after BvB win");
        }
    }
}
