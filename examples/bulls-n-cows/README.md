# WhatsApp Bulls & Cows Bot Example

This project is an example bot for the `whatsapp-business-rs` crate. It implements a fully interactive, turn-based "Bulls & Cows" (or "Call & Said") game that can be played via WhatsApp messages.

The bot features:
* Multiple game modes (Human vs. Bot, Bot vs. Bot, Assist Mode).
* A robust, per-user state machine.
* A clean separation of state logic (`main.rs`) from UI/parsing logic (`helpers.rs`).
* Efficient, thread-safe state management using `dashmap`.

## The "Immediate Mode" State Machine Pattern

This code borrows a core concept from **Immediate Mode GUIs** (like `egui` or `Dear ImGui`).

In a traditional "retained mode" GUI, you create objects (like buttons) and attach event listeners (like `onClick`) to them. The state is spread out across many different callbacks.

In an "immediate mode" GUI, you re-draw the entire UI from scratch every single frame. The "state" is held centrally, and the UI is just a *function* of that state.

**How this applies here:**

* **"Frame"** = A new incoming message from a user.
* **"Input"** = The content of that message (e.g., text, or a button click).
* **"Central State"** = The `Room` struct for that specific user.
* **"Draw Loop"** = The `Room::update` method.

Every time a message arrives, we call `room.update(msg)`. This single function is responsible for:
1.  Looking at the **current state** (e.g., `RoomState::Start`).
2.  Delegating to the update logic for *that specific state* (e.g., `Room::update_start`).
3.  The state-specific logic parses the **input** (the `msg`).
4.  It sends any necessary replies (the "UI").
5.  It then returns `Some(new_state)` to **transition** the state, or `None` to **stay in the same state**.

This pattern is not standard for webhooks, but it's incredibly smooth and easy to reason about for turn-based games. All the logic for a given state (like `RoomState::Game`) is co-located, and state transitions are explicit, simple, and clean.

## Core Components

### `main.rs`: The State Machine

This file contains the "brain" of the application. It cares about *what* state the user is in and *what* to do next.

* **`main` function**: Sets up the `Config`, builds the `whatsapp_business_rs::Server` and `Client`, registers the webhook with Meta, and starts the server.
* **`App`**: The main struct that implements `WebhookHandler`. Its *only* job is to receive messages and find the correct `Room`.
* **`Rooms`**: A type-safe wrapper for `Arc<DashMap<String, Room>>`. This is our globally shared, thread-safe "database" of all active game sessions.
* **`Room`**: Represents a single user's session. This is the **top-level state machine** and is the heart of the application.
* **`RoomState`**: The top-level state enum: `Start`, `Game(GameMode)`, `End(End)`.
* **`GameMode`**: A **nested state machine** enum: `BvB`, `HvB`, `Assist`.
* **`BotVsBot`, `HumanVsBot`, `AssistMode`**: Structs that hold the specific state for each game mode.

### `helpers.rs`: The "View" and "Controller"

This file contains the "View" (UI) and "Controller" (Input Parsing) logic. It knows *how* to talk to the user and *how* to understand them, but it holds no state of its own. This creates a clean **Separation of Concerns**.

It provides two main categories of helpers:

1.  **Action Parsers**:
    * Example: `HvbAction::parse(msg)`.
    * These functions take a raw `IncomingMessage` and translate it into a semantic enum (an "Action"), like `HvbAction::Both { grade, call }`.
    * This is where all the messy input logic lives, like parsing text with **Regex** (`HvbAction::parse_str`) or checking button `callback` payloads.
    * The state machine in `main.rs` doesn't care *how* the action was parsed; it just `match`es on the clean `HvbAction` enum.

2.  **Reply Functions (Views)**:
    * Example: `HvbAction::send_welcome(msg)`.
    * These are functions that construct and send WhatsApp messages using the `Draft` builder.
    * They contain all the user-facing text, emojis, and button definitions.
    * Many reply functions use helpers like `HvbAction::bot_commentary` to add "flavor text" and make the bot feel more alive.
    * **Builder Pattern**: For complex, multi-stage replies (like the `BvbAction`), a builder pattern (`BvbActionResponseBuilder`) is used to construct the response step-by-step.
    * **"Mounting" Pattern**: In-game reply functions (like `HvbAction::reply_invalid`) often call `GameRoomAction::reply_on(msg, ...)`. This is a wrapper that "mounts" the generic game-room buttons (Help, Restart, Menu) onto the specific message, providing a consistent "escape hatch" for the user from any game state.

## Code Walkthrough: The Lifecycle of a Message

Let's follow a user, "Alex," as he plays a game.

### 1. The First Message: "Hi"

1.  Alex sends "Hi". Meta sends a webhook to our server.
2.  The `Server` receives it and calls `App::handle_message`.
3.  `App::handle_message` gets Alex's phone ID (`room_id`).
4.  It calls `self.get_or_start_room(room_id)`.
    * The `DashMap` in `self.rooms` has no entry for Alex.
    * `DashMap::entry().or_default()` creates a new, default `Room`.
    * The default `Room` has its state set to `RoomState::Start`.
5.  `App::handle_message` calls `room.update(msg)` on Alex's new `Room`.
6.  `Room::update` (in `main.rs`) matches on `self.state`, which is `RoomState::Start`. It calls `Room::update_start(msg)`.
7.  `Room::update_start` (in `main.rs`) calls `StartAction::parse(&msg)` (from `helpers.rs`). This parses "Hi" into `StartAction::Hi`.
8.  It matches `StartAction::Hi` and calls `StartAction::send_hello(&msg)` (from `helpers.rs`), which sends the "Welcome! Choose a game mode..." message with buttons.
9.  `Room::update_start` returns `None`, because the state hasn't changed. Alex is still at the main menu.
10. `Room::update` sees `None` and does nothing. The `Room`'s state remains `RoomState::Start`.

### 2. Starting a Game: Clicks "Human vs Bot"

1.  Alex clicks the "Human vs Bot" button. Meta sends a webhook.
2.  Steps 2-5 repeat. `get_or_start_room` finds Alex's *existing* `Room`, which is still in `RoomState::Start`.
3.  `Room::update` calls `Room::update_start(msg)`.
4.  `Room::update_start` calls `StartAction::parse(&msg)`. The button click payload `START_HVB_CALLBACK` is parsed into `StartAction::Start { mode: GameModeRef::HvB }`.
5.  It matches `StartAction::Start` and calls `mode.start(&msg)`, which is `GameModeRef::HvB.start(&msg)`.
6.  This calls `HumanVsBot::start(msg)` (in `main.rs`).
    * A new `HumanVsBot` struct is created (with a random bot password).
    * `HvbAction::send_welcome(msg)` (from `helpers.rs`) sends the "Game started! Make your guess." message.
    * `HumanVsBot::start` returns `RoomState::Game(GameMode::HvB(me))`.
7.  `Room::update_start` returns `Some(RoomState::Game(GameMode::HvB(me)))`.
8.  `Room::update` sees `Some(new_state)`. It executes `self.state = new_state`.
9.  **The state transition is complete.** Alex's `Room` is now in the `RoomState::Game` state.

### 3. Gameplay: "1b 0c 4321" (Grade and Guess)

1.  Alex sends "1b 0c 4321". This grades the bot's previous guess and makes a new guess.
2.  Steps 2-5 repeat. `get_or_start_room` finds Alex's `Room`, which is in `RoomState::Game`.
3.  `Room::update` (in `main.rs`) matches on `RoomState::Game(game)`. It calls `game.update(msg)`.
4.  `GameMode::update` (in `main.rs`) checks for generic commands (it's not one) and delegates to `human_vs_bot.update(msg)`.
5.  `HumanVsBot::update` (in `main.rs`) calls `HvbAction::parse(&msg)` (from `helpers.rs`).
6.  `HvbAction::parse` calls `HvbAction::parse_str`. The regexes find both a grade (`1d 0c`) and a call (`4321`). It returns `HvbAction::Both { grade, call }`.
7.  `HumanVsBot::update` matches `HvbAction::Both` and calls `self.handle_both(msg, grade, call)`.
8.  `HumanVsBot::handle_both` (in `main.rs`):
    * First, it calls `self.try_settle_grade(msg, grade)` to process the grade.
    * Second, it calls `self.handle_call(msg, call)` to process the new guess.
    * `handle_call` scores the user's guess, makes the bot's next guess, and (assuming no winner) calls `HvbAction::send_grade_n_call(...)` (from `helpers.rs`) to send the reply.
    * `handle_call` returns `None`.
9.  All `update` functions return `None`. The state remains `Game`.

### 4. Winning and Sharing

1.  Alex finally guesses the bot's password: "5678".
2.  The logic drills down to `HumanVsBot::handle_call` (in `main.rs`).
3.  `user_score.is_win()` is **true**.
4.  It calls `self.end_human_wins(msg)`.
5.  `end_human_wins` (in `main.rs`) calls `HvbAction::get_share_win_message(msg, rounds)` (from `helpers.rs`) to get the shareable text.
6.  It then calls `End::with_win_msg(msg, mode, win_msg)` (in `main.rs`), which sends the "You won! Play Again? / Share Win" buttons.
7.  `end_human_wins` returns `Some(RoomState::End(me))`.
8.  `Room::update` sees this `Some` and transitions `self.state = RoomState::End(me)`.
9.  Alex is now in the `End` state.

### 6. Playing Again

1.  Alex clicks the "Play Again" button.
2.  `App::handle_message` finds Alex's `Room`, which is in `RoomState::End`.
3.  `Room::update` calls `end.update(msg)`.
4.  `End::update` parses the button click as `EndAction::PlayAgain`.
5.  It calls `self.last.start(&msg)`. (`self.last` was saved as `GameModeRef::HvB`).
6.  This calls `HumanVsBot::start(msg)`, which creates a *brand new game* and returns `RoomState::Game(GameMode::HvB(new_game))`.
7.  `End::update` returns `Some(RoomState::Game(...))`.
8.  `Room::update` transitions `self.state` back to `Game`. The loop is complete.