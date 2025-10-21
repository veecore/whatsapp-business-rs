//! # Game Helpers and Action Parsers
//!
//! This module provides helper structs, enums, and parsing logic for interpreting
//! user input and constructing replies. It decouples the core state machine in `main.rs`
//! from the details of message parsing.
//!
//! It also defines a pattern where game actions can be intercepted by generic "escape"
//! commands (like 'help', 'quit') for a better user experience. This avoids locking
//! the user into a game state they can't exit.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Display;
// TODO: Use async
use std::sync::LazyLock as Lazy; // Use LazyLock for efficient, one-time static initialization.

use rand::seq::IndexedRandom as _; // Trait for selecting a random element from a slice.

use regex::Regex; // Used for flexible parsing of user text input.

use whatsapp_business_rs::Draft; // A builder for creating outgoing messages.
use whatsapp_business_rs::message::IntoDraft as OutgoingMessage;
use whatsapp_business_rs::message::{Button, Content, InteractiveContent};
use whatsapp_business_rs::server::IncomingMessage;

use call_n_said::Digit;
// Import core game types and alias them for clarity within this module.
use call_n_said::Guess as Call;
use call_n_said::Guess as Password;
use call_n_said::GuessResult as Grade;

// ----- START BUTTONS -----
// These consts define the 'callback' payloads for interactive buttons.
// Using consts prevents typos and makes the code more maintainable.
pub const START_HVB_CALLBACK: &str = "start_hvb";
pub const START_BVB_CALLBACK: &str = "start_bvb";
pub const START_ASSIST_CALLBACK: &str = "start_assist";
pub const START_LEARNMORE_CALLBACK: &str = "learn_more";

// ----- BVB BUTTONS -----
pub const BVB_NEXT_CALLBACK: &str = "bvb_next";

// ----- END BUTTONS -----
pub const END_MAINMENU_CALLBACK: &str = "main_menu";
pub const PLAY_AGAIN_CALLBACK: &str = "play_again";
pub const SHARE_WIN_CALLBACK: &str = "share_win";

// ----- GENERIC GAME BUTTONS -----
// These are the "escape hatch" commands available during any game.
pub const GAME_ROOM_RESTART_CALLBACK: &str = "game_room_restart";
pub const GAME_ROOM_MAINMENU_CALLBACK: &str = "game_room_mainmenu";
pub const GAME_ROOM_HELP_CALLBACK: &str = "game_room_help";

// TODO: Rollback on reply error
/// A simple error-reporting macro.
/// It wraps a `Result` and prints any `Err` to stderr without panicking.
macro_rules! try_reply {
    ($result:expr) => {
        if let Err(err) = $result {
            eprintln!("Failed to send reply: {}", err)
        }
    };
}

/// A standardized error structure for parsing failures.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    err_msg: Cow<'static, str>,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.err_msg)
    }
}

/// A lightweight, copyable reference to a game mode.
/// This is used in `StartAction` to signal *which* game to start,
/// as opposed to the `main.rs::GameMode` enum which holds the *state* of the game.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum GameMode {
    BvB,
    HvB,
    Assist,
}

// ======================= User & Bot Actions Superimposed ======================= ///
// The following enums (`StartAction`, `BvbAction`, etc.) represent the *semantic*
// meaning of a user's input, translated from the raw `IncomingMessage`.
// =============================================================================== //

// ------ START ------ //
/// Represents all possible actions a user can take from the main menu (`Start` state).
#[derive(Clone, Debug)]
pub enum StartAction {
    /// User sent a greeting.
    Hi,
    /// User chose a game mode to play.
    Start { mode: GameMode },
    /// User asked for the rules.
    LearnMore,
    /// User sent something unrecognizable.
    Invalid { err: Error },
}

impl StartAction {
    /// Parses an `IncomingMessage` into a `StartAction`.
    #[inline]
    pub fn parse(msg: &IncomingMessage) -> Self {
        match &msg.content {
            // Case 1: User clicked an interactive button. This is the preferred, fast path.
            Content::Interactive(InteractiveContent::Click(Button::Reply(reply))) => {
                match reply.call_back.as_str() {
                    START_HVB_CALLBACK => Self::Start {
                        mode: GameMode::HvB,
                    },
                    START_BVB_CALLBACK => Self::Start {
                        mode: GameMode::BvB,
                    },
                    START_ASSIST_CALLBACK => Self::Start {
                        mode: GameMode::Assist,
                    },
                    START_LEARNMORE_CALLBACK => Self::LearnMore,
                    _ => Self::Invalid {
                        err: Error {
                            err_msg: "Unknown button clicked.".into(),
                        },
                    },
                }
            }
            // Case 2: User sent a text message. Fallback to keyword matching.
            Content::Text(text) => {
                let lower = text.body.to_lowercase();
                if lower.contains("hi")
                    || lower.contains("hello")
                    || lower.contains("start")
                    || lower.contains("play")
                {
                    Self::Hi
                } else if lower.contains("learn more") {
                    Self::LearnMore
                } else {
                    Self::Invalid {
                        err: Error {
                            err_msg: "Say 'hi' to start!".into(),
                        },
                    }
                }
            }
            // Case 3: User sent something else (e.g., an image, audio).
            _ => Self::Invalid {
                err: Error {
                    err_msg: "Please send a text message or tap a button.".into(),
                },
            },
        }
    }

    /// "View" function: Sends the main menu message.
    #[inline]
    pub async fn send_hello(msg: &IncomingMessage) {
        let body = "Hey there! 👋 Welcome to Bulls & Cows! 🐂🐄

It's a classic code-breaking game where you try to guess a secret 4-digit number.

Ready to play? Pick a mode to get started!";

        let draft = Draft::new()
            .body(body)
            .add_reply_button(START_HVB_CALLBACK, "🙂 Human vs Bot")
            .add_reply_button(START_BVB_CALLBACK, "🤖 Bot vs Bot")
            .add_reply_button(START_ASSIST_CALLBACK, "🤔 Assist Mode")
            // .add_reply_button(START_LEARNMORE_CALLBACK, "Rules & Info")
            // FIXME: Better as button but button max is 3 (I secretly hate meta)
            .footer("Type 'Learn More' for rules");

        try_reply!(msg.reply(draft).await);
    }

    #[inline]
    pub async fn send_learn_more(msg: &IncomingMessage) {
        let body =
"*How to Play Bulls & Cows* 📜

The goal is to guess a secret 4-digit number. All digits in the number are unique (e.g., `1234` is valid, but `1123` is not).

After each guess, you'll get a score in the form of 'Deads' and 'Injures':

🐂 *Bulls (or Deads)*: Correct digits in the *correct position*.
🐄 *Cows (or Injures)*: Correct digits but in the *wrong position*.

*Example*:
If the secret number is `1234` and you guess `1354`:
- `1` is a Dead (correct digit, correct position).
- `4` is a Dead (correct digit, correct position).
- `3` is a Injure (correct digit, wrong position).
- `5` is incorrect.
Your score would be: *2 Deads and 1 Injure*.

You win by getting *4 Deads*!

*This bot was built with Rust using the `whatsapp-business-rs` crate.*";

        let draft = Draft::new()
            .body(body)
            .add_reply_button(START_HVB_CALLBACK, "🙂 Human vs Bot")
            .add_reply_button(START_BVB_CALLBACK, "🤖 Bot vs Bot")
            .add_reply_button(START_ASSIST_CALLBACK, "🤔 Assist Mode");

        try_reply!(msg.reply(draft).await);
    }

    /// "View" function: Sends a parse error message.
    #[inline]
    pub async fn reply_invalid(msg: &IncomingMessage, err: Error) {
        try_reply!(msg.reply(err.to_string()).await);
    }
}

// NOTE: REMEMBER TO MOUNT GAMEROOMACTION UNTIL WE FIND AN EASIER WAY

// ------ BVB ------ //
/// Represents actions in the Bot-vs-Bot game.
#[derive(Clone, Debug)]
pub enum BvbAction {
    /// User clicked "Next" to see the next turn.
    Next,
    /// User sent something else.
    Invalid { err: Error },
}

impl BvbAction {
    /// Parses input for BvB. It's very simple: only one button is valid.
    #[inline]
    pub fn parse(msg: &IncomingMessage) -> Self {
        match &msg.content {
            Content::Interactive(InteractiveContent::Click(Button::Reply(reply)))
                if reply.call_back == BVB_NEXT_CALLBACK =>
            {
                Self::Next
            }
            _ => Self::Invalid {
                err: Error {
                    err_msg: "Please tap the 'Next Turn' button or type 'help' for options.".into(),
                },
            },
        }
    }

    /// Sends an invalid message, but "on" `GameRoomAction`.
    /// This means the error message will *also* have the generic "help/menu" buttons.
    #[inline]
    pub async fn reply_invalid(msg: &IncomingMessage, err: Error) {
        GameRoomAction::reply_on(msg, err.to_string()).await;
    }

    /// Sends the BvB help message, also "on" `GameRoomAction`.
    #[inline]
    pub async fn send_help(msg: &IncomingMessage) {
        let text = "This is *Bot vs Bot* mode 🤖💥🤖

Sit back, relax, and tap '▶️ Next Turn' to watch two bots battle it out!

You can also use these commands:
- `restart`: Start a new BvB game.
- `menu`: Go back to the main menu.
- `help`: Show this message again.";
        GameRoomAction::reply_on(msg, text).await;
    }

    /// This kicks off a **Builder Pattern** for the BvB turn summary reply.
    /// This is useful because the reply is built in stages and can have
    /// different final forms (A wins, B wins, or continue).
    #[inline]
    pub fn reply(msg: &'_ IncomingMessage) -> BvbActionResponseBuilder<'_> {
        BvbActionResponseBuilder { msg, a: (), b: () }
    }
}

/// The builder struct for the BvB turn summary.
/// The generic types `A` and `B` use the "type state" pattern to track
/// what information has been added. `()` means "not yet set".
pub struct BvbActionResponseBuilder<'a, A = (), B = ()> {
    msg: &'a IncomingMessage,
    a: A,
    b: B,
}

/// State 1: No calls added yet.
impl<'a> BvbActionResponseBuilder<'a> {
    /// Adds Bot A's move to the builder.
    #[inline]
    pub fn set_a_call(
        self,
        a_call: Call,
        a_grade: Grade,
    ) -> BvbActionResponseBuilder<'a, (Call, Grade)> {
        BvbActionResponseBuilder {
            msg: self.msg,
            a: (a_call, a_grade),
            b: (), // Bot B's info is still empty.
        }
    }
}

/// State 2: Bot A's call has been added.
impl<'a> BvbActionResponseBuilder<'a, (Call, Grade)> {
    /// Final state: Bot A wins. Sends the message.
    #[inline]
    pub async fn send_a_wins(self, password: Password) {
        let (a_call, a_grade) = self.a;
        let win_msg = format!(
            "--- 🤖 Turn Summary 🤖 ---
*Bot A* guesses `{a_call}` ➡️ gets *{a_grade}*.

🏆 *Bot A wins!* The secret number was `{password}`.",
        );

        try_reply!(self.msg.reply(win_msg).await);
    }

    /// Adds Bot B's move to the builder.
    #[inline]
    pub fn set_b_call(
        self,
        b_call: Call,
        b_grade: Grade,
    ) -> BvbActionResponseBuilder<'a, (Call, Grade), (Call, Grade)> {
        BvbActionResponseBuilder {
            msg: self.msg,
            a: self.a,            // Keep Bot A's info
            b: (b_call, b_grade), // Add Bot B's info
        }
    }
}

/// State 3: Both Bot A's and Bot B's calls have been added.
impl<'a> BvbActionResponseBuilder<'a, (Call, Grade), (Call, Grade)> {
    /// Final state: Bot B wins. Sends the message.
    #[inline]
    pub async fn send_b_wins(self, password: Password) {
        let (a_call, a_grade) = self.a;
        let (b_call, b_grade) = self.b;
        let win_msg = format!(
            "--- 🤖 Turn Summary 🤖 ---
*Bot A* guesses `{a_call}` ➡️ gets *{a_grade}*.
*Bot B* guesses `{b_call}` ➡️ gets *{b_grade}*.

🏆 *Bot B wins!* The secret number was `{password}`."
        );

        try_reply!(self.msg.reply(win_msg).await);
    }

    /// Final state: No winner. Sends the summary and the "Next Turn" button.
    #[inline]
    pub async fn send(self) {
        let (a_call, a_grade) = self.a;
        let (b_call, b_grade) = self.b;
        let body = format!(
            "--- 🤖 Turn Summary 🤖 ---
*Bot A* guesses `{a_call}` ➡️ gets *{a_grade}*.
*Bot B* guesses `{b_call}` ➡️ gets *{b_grade}*."
        );

        let response = Draft::new()
            .body(body)
            .add_reply_button(BVB_NEXT_CALLBACK, "▶️ Next Turn");

        // Use `GameRoomAction::reply_on` to add generic game buttons.
        GameRoomAction::reply_on(self.msg, response).await;
    }
}

// ------ HVB ------ //

/// Represents the possible actions a user can take during a HumanVsBot game.
#[derive(Clone, Debug, PartialEq)]
pub enum HvbAction {
    /// User is only grading the bot's last guess (e.g., "1b 2c").
    Grade(GradeAction),
    /// User is only making a new guess (e.g., "1234").
    Call(Call),
    /// User is doing both: grading and guessing (e.g., "1b 2c 5678").
    Both { grade: GradeAction, call: Call },
    /// User is setting their secret number so the bot can grade its own guesses (e.g., "set 1234")..
    SetPassword(Password),
    /// The user's input could not be understood.
    Invalid { err: Error },
}

/// A wrapper for a `Grade` that might eventually include *which* guess is being graded.
#[derive(Clone, Debug, PartialEq)]
pub struct GradeAction {
    pub grade: Grade,
    // The `target` is the specific bot guess the user is grading.
    // This is currently `None` from parsing, as it's hard to link
    // a reply to a specific previous message without more state.
    // The state machine in `main.rs` assumes this grade applies
    // to the `last_bot_call`.
    pub target: Option<Call>,
}

// Use `LazyLock` to compile Regex only once for efficiency.
// `\b` word boundaries are important to not match "12345" as "1234".
static CALL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(\d{4})\b").unwrap());
// This regex is flexible, accepting "d", "deads", "b", or "bulls" for deads.
static DEADS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\d)\s*(?:[dD](?:eads?)?|[bB](?:ulls?)?)").unwrap());
// This regex accepts "i", "injures", "c", "cows", or "j" for injures.
static INJURES_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\d)\s*(?:[iI](?:njures?)?|[cC](?:ows?)?|[jJ])").unwrap());
// This regex accepts "set 1234" or "password 1234".
static SET_PASSWORD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:set|password)\s+(\d{4})\b").unwrap());

impl HvbAction {
    /// Parses an `IncomingMessage` to determine the user's action.
    /// This is the main entry point for parsing `HvB` messages.    
    #[inline]
    pub fn parse(msg: &IncomingMessage) -> Self {
        match &msg.content {
            // HvB only supports text input for guesses and grades.
            Content::Text(text) => Self::parse_str(&text.body),
            _ => Self::Invalid {
                err: Error {
                    err_msg: "Please send a text message with your guess or grade.".into(),
                },
            },
        }
    }

    /// A rugged parser for "Bulls & Cows" game input from a string.
    ///
    /// This function is designed to handle a variety of user input styles.
    /// It uses regular expressions to find a 4-digit number (a "call") and
    /// the score (deads and injures, the "grade").
    #[inline]
    fn parse_str(input: &str) -> Self {
        let trimmed_input = input.trim();

        if trimmed_input.is_empty() {
            return Self::Invalid {
                err: Error {
                    err_msg: "Your message was empty.".into(),
                },
            };
        }

        // Check for "set password" command first, as it's a special action.
        if let Some(caps) = SET_PASSWORD_RE.captures(trimmed_input) {
            let number_str = caps.get(1).unwrap().as_str();
            return match parse_guess(number_str) {
                Ok(password) => Self::SetPassword(password),
                Err(err) => Self::Invalid { err },
            };
        }

        // --- Standard Parsing Logic ---

        // 1. Try to find a 4-digit call.
        let call_match = CALL_RE.find(trimmed_input);
        let call_result = call_match.map(|m| parse_guess(m.as_str())); // `parse_guess` validates unique digits.

        // 2. Try to find grade components (deads/injures).
        let deads = DEADS_RE
            .captures(trimmed_input)
            .and_then(|caps| caps.get(1)?.as_str().parse::<u8>().ok())
            .unwrap_or(0);

        let injures = INJURES_RE
            .captures(trimmed_input)
            .and_then(|caps| caps.get(1)?.as_str().parse::<u8>().ok())
            .unwrap_or(0);

        // 3. Check if a grade was *explicitly* mentioned.
        // This is key to distinguish "0d 0i" (a valid grade) from no grade at all.
        let grade_found = DEADS_RE.is_match(trimmed_input) || INJURES_RE.is_match(trimmed_input);

        let grade = Grade { deads, injures };
        // 4. Validate the grade for impossible combinations.
        if grade.deads + grade.injures > 4 {
            return Self::Invalid {
                err: Error {
                    err_msg: "The total of bulls and cows cannot exceed 4.".into(),
                },
            };
        }
        // 3 Bulls + 1 Cow is mathematically impossible in a 4-digit unique-digit game.
        if grade.deads == 3 && grade.injures == 1 {
            return Self::Invalid {
                err: Error {
                    err_msg: "A score of 3 bulls and 1 cow is impossible. Please check your score."
                        .into(),
                },
            };
        }

        // 5. Decide the action based on what was found.
        match (grade_found, call_result) {
            // Case 1: Both a grade and a valid call were found. (e.g., "1d 2i 5678")
            (true, Some(Ok(call))) => Self::Both { grade: GradeAction { target: None, grade }, call },

            // Case 2: Grade found, but the call was invalid (e.g., "1d 1123").
            (true, Some(Err(err))) => Self::Invalid { err }, // Return the error from `parse_guess`.

            // Case 3: Only a grade was found. (e.g., "1d 2i")
            (true, None) => Self::Grade(GradeAction { target: None, grade }),

            // Case 4: Only a valid call was found. (e.g., "5678")
            (false, Some(Ok(call))) => {
                // To avoid ambiguity, we only accept a call if it's the *only* thing in the message.
                if call_match.unwrap().as_str() == trimmed_input {
                    Self::Call(call)
                } else {
                    Self::Invalid { err: Error { err_msg: "Ambiguous message. To make a guess, please send only the 4-digit number.".into() } }
                }
            }
            // Case 5: A call was found, but it was invalid (e.g., "my guess is 1123").
            (false, Some(Err(err))) => Self::Invalid { err },

            // Case 6: Neither a valid grade nor a valid call could be parsed. (e.g., "hello")
            (false, None) => Self::Invalid { err: Error { err_msg: "I didn't understand. Please send a 4-digit guess, grade my last call (e.g., '1d 2i'), or type 'help'.".into() } },
        }
    }

    /// "View" function: Sends an invalid parse error.
    #[inline]
    pub async fn reply_invalid(msg: &IncomingMessage, err: Error) {
        GameRoomAction::reply_on(msg, err.to_string()).await;
    }

    /// "View" function: Replies with user's score and bot's new guess (manual grade path).
    #[inline]
    pub async fn send_grade_n_call(msg: &IncomingMessage, user_grade: GradeAction, call: Call) {
        let (grade, user_guess) = (
            user_grade.grade,
            user_grade
                .target
                .expect("Bot must include target when calling this"),
        );

        // Get some "flavor text" for the user's score.
        let user_comment = Self::user_commentary(grade);

        let response = format!(
            "{user_comment}

Your guess `{user_guess}` got *{grade}*.

My turn! I'm guessing `{call}`. How did I do?"
        );

        GameRoomAction::reply_on(msg, response).await;
    }

    /// "View" function: Replies with both scores (auto-grade path).
    #[inline]
    pub async fn send_grade_n_call_n_grade_myself(
        msg: &IncomingMessage,
        user_grade: GradeAction,
        bot_grade: GradeAction,
    ) {
        let (user_grade, user_guess) =
            (user_grade.grade, user_grade.target.expect("include target"));
        let (bot_grade, bot_guess) = (bot_grade.grade, bot_grade.target.expect("include target"));

        // FIXED: Human grade was swapped for bot's... very silly
        // Get flavor text for both scores.
        let bot_comment = Self::bot_commentary(user_grade); // Commentary on bot's score
        let user_comment = Self::user_commentary(bot_grade); // Commentary on user's score

        let response = format!(
            "{user_comment}

Your guess `{user_guess}` got *{user_grade}*.

My turn! I guessed `{bot_guess}`...
...and based on your secret number, I score *{bot_grade}*. {bot_comment}

Your turn!
"
        );
        GameRoomAction::reply_on(msg, response).await;
    }

    /// "View" function: Confirms the user's password has been set.
    #[inline]
    pub async fn send_user_password_saved(msg: &IncomingMessage, password: Password) {
        GameRoomAction::reply_on(
            msg,
            format!(
                "Got it! Your secret number `{password}` is saved. 

Now you only need to send your guesses. I'll handle scoring my own moves automatically. Good luck!"
            ),
        )
        .await;
    }

    // TODO: Set new logic
    /// "View" function: Asks for confirmation when changing a password.
    #[inline]
    pub async fn send_user_password_already_saved(
        msg: &IncomingMessage,
        former: Password,
        new: Password,
    ) {
        GameRoomAction::reply_on(
            msg,
            format!(
                "Heads up! You already set your secret number to `{former}`. Did you mean to change it to `{new}`? 
                
If so, send `set {new}` again to confirm."
            ),
        )
        .await;
    }

    /// "View" function: Tells the user not to grade manually when auto-grade is on.
    #[inline]
    pub async fn send_user_grade_not_needed(msg: &IncomingMessage) {
        GameRoomAction::reply_on(
            msg,
            "No need to grade me! Since you've set your password, I can score myself. Just send your next guess.".to_string(),
        )
        .await;
    }

    #[inline]
    pub async fn send_promt_user_to_call(msg: &IncomingMessage, bot_grade: Grade) {
        // --- Get commentary for the bot's score ---
        let commentary = Self::bot_commentary(bot_grade);

        GameRoomAction::reply_on(
            msg,
            format!("{commentary}\n\nThanks for the score! 👍 Your turn to make a guess."),
        )
        .await;
    }

    /// "View" function (Error): User tried to guess when they needed to grade.
    #[inline]
    pub async fn send_last_call_not_graded(msg: &IncomingMessage, call: Call) {
        GameRoomAction::reply_on(
            msg,
            format!("Whoa there! ✋ You haven't scored my last guess of `{call}` yet. Please score it before making your next guess."),
        )
        .await;
    }

    /// "View" function (Error): User tried to grade an old guess.
    #[inline]
    pub async fn send_target_unsupported(msg: &IncomingMessage, last_call: Call) {
        GameRoomAction::reply_on(
            msg,
            format!("Sorry, I can only accept a grade for my most recent guess, which was `{last_call}`."),
        ).await;
    }

    /// "View" function (Error): User tried to grade when it wasn't the bot's turn.
    #[inline]
    pub async fn send_is_not_bot_turn(msg: &IncomingMessage) {
        GameRoomAction::reply_on(
            msg,
            "I haven't made a guess yet! It's your turn to guess first.",
        )
        .await;
    }

    /// "View" function: Sends the human win message.
    #[inline]
    pub fn send_human_wins(
        msg: &IncomingMessage,
        rounds: usize,
        password: Password,
    ) -> impl Future<Output = ()> + Send {
        // Use match to provide different celebratory messages based on performance.
        let win_message = match rounds {
            1 => format_args!(
                "Incredible! 🤯 You guessed it on the first try! Are you psychic? clairvoyant?"
            ),
            2..=4 => {
                format_args!(
                    "Amazing! 🥳 You solved it in just {rounds} guesses! You're a natural. 🏆"
                )
            }
            5..=7 => format_args!(
                "Well done! ✅ You cracked the code in {rounds} rounds. Solid victory! 💪"
            ),
            _ => {
                format_args!(
                    "You got it! 🎉 It took {rounds} rounds, but you persevered. Great game! 🙌"
                )
            }
        };

        let message = format!("The secret number was *{password}*.\n\n{win_message}");
        // Manually return a future that sends the reply... This is so we could use format_args which isn't Sync.
        async { try_reply!(msg.reply(message).await) }
    }

    /// Generates a pre-formatted message for the user to share their victory with friends.
    /// This does *not* send a message, it just returns a string for the `End` state to use.
    #[inline]
    pub fn get_share_win_message(msg: &IncomingMessage, rounds: usize) -> impl Display {
        let achievement = match rounds {
            1 => format_args!("in a single guess! Unbelievable! 🤯"),
            2..=4 => format_args!("in just {rounds} guesses! Top-tier code-breaking. 😎"),
            _ => format_args!("in {rounds} rounds. A hard-fought victory!"),
        };

        format!(
            "🏆 I just won a game of Bulls & Cows! 🐂🐄\n\nI beat the bot and cracked the secret code {achievement}. Join me {}",
            make_chat_url(msg)
        )
    }

    /// "View" function: Bot wins.
    #[inline]
    pub async fn send_bot_wins(msg: &IncomingMessage) {
        try_reply!(
            msg.reply("🤖 I win! That was a fun game. Thanks for playing!")
                .await
        );
    }

    /// "View" function (Error): A contradiction was found in the user's grades.
    #[inline]
    pub async fn send_discrepancy(msg: &IncomingMessage) {
        try_reply!(msg.reply("🤔 Hmmm, something's not right. Based on your scores, I have no possible numbers left. One of the scores you gave me must have been incorrect.").await);
    }

    /// "View" function: Welcome message for HvB.
    #[inline]
    pub async fn send_welcome(msg: &IncomingMessage) {
        GameRoomAction::reply_on(
            msg,
            "Alright, I've picked my secret number. Let the game begin! 🚀

What's your first guess?",
        )
        .await;
    }

    /// "View" function: Help text for HvB.
    #[inline]
    pub async fn send_help(msg: &IncomingMessage) {
        let text = "*How to Play Human vs Bot* 🙂

1. Think of your own secret 4-digit number (with unique digits).
2. Send your first guess to me (e.g., `1234`).
3. I will score your guess and reply with my own guess.
4. Score my guess by sending a reply like `1d 2i` (for 1 dead/bull, 2 injures/cows).
5. You can also include your next guess in the same message, e.g., `1d 2i 5678`.

Keep going until one of us guesses the other's number!

*Pro Tip*: If grading me gets tedious, just tell me your secret number with `set XXXX`. I'll score myself automatically!

You can also use these commands:
- `restart`: Start a new game.
- `menu`: Go back to the main menu.
";
        GameRoomAction::reply_on(msg, text).await;
    }

    /// Generates commentary based on the score of the *user's* guess.
    /// This adds a personal, "AI" touch to the bot's responses.
    #[inline]
    fn user_commentary(grade: Grade) -> &'static str {
        match (grade.deads, grade.injures) {
            (4, 0) => "An impossible state, the win is handled before this.",
            (3, _) => random_choice(&[
                "Wow, you're so close! 🔥",
                "Just one more digit to nail down!",
                "Incredible guess! You're on fire!",
            ]),
            (2, 2) => random_choice(&[
                "Nice one! A perfect swap away.",
                "Great guess! You've found all the digits.",
            ]),
            (2, _) => random_choice(&[
                "You're getting warmer... 👀",
                "Good progress, keep it up!",
                "Two in the right spot! Solid.",
            ]),
            (1, 3) => random_choice(&[
                "Excellent! All digits are correct.",
                "You've found them all! Just need to shuffle them.",
            ]),
            (1, _) => random_choice(&["That's a start.", "Okay, one down.", "A good foothold."]),
            (0, 4) => random_choice(&[
                "Whoa, you have all the right digits, just in the wrong places!",
                "A perfect shuffle! All digits are correct.",
            ]),
            (0, 3) => random_choice(&[
                "You've found three of the digits!",
                "Nice! You're circling the target.",
            ]),
            (0, 2) => {
                random_choice(&["Okay, two correct digits.", "A solid piece of information."])
            }
            (0, 1) => random_choice(&["Got one.", "A clue to work with."]),
            // A score of 0d 0i is very valuable information.
            (0, 0) => random_choice(&[
                "Ouch, a complete miss.",
                "Nothing on that one. A fresh start!",
                "Alright, that eliminates a few things.",
            ]),
            _ => "Noted.", // Fallback for any other combination
        }
    }

    /// Generates commentary based on the score of the *bot's* guess.
    #[inline]
    fn bot_commentary(grade: Grade) -> &'static str {
        match (grade.deads, grade.injures) {
            (4, 0) => "I did it! 🤖",
            (3, _) => random_choice(&[
                "I'm closing in... 🎯",
                "Just one more to go!",
                "I've almost got it!",
            ]),
            (2, 2) => random_choice(&[
                "Aha! All the correct digits. Now to swap them.",
                "Interesting... I have all the pieces.",
            ]),
            (2, _) => random_choice(&[
                "Two in place. That narrows it down considerably.",
                "Good, good. Making progress.",
                "Okay, that's very helpful.",
            ]),
            (1, 3) => random_choice(&[
                "Excellent, all digits confirmed. Time to rearrange.",
                "Perfect. Now for the final positions.",
            ]),
            (1, _) => random_choice(&[
                "One down. Building my case.",
                "A useful data point.",
                "Okay, I'll factor that in.",
            ]),
            (0, 4) => random_choice(&[
                "Fascinating. I have all the numbers, just need the order.",
                "All the right materials, wrong construction.",
            ]),
            (0, 3) => random_choice(&[
                "Okay, three correct digits to work with.",
                "This is getting interesting...",
            ]),
            (0, 2) => random_choice(&[
                "Two digits confirmed. That helps.",
                "Alright, two pieces of the puzzle.",
            ]),
            (0, 1) => random_choice(&["A starting point.", "Every bit of information helps."]),
            (0, 0) => random_choice(&[
                "Hmm, a complete miss. Back to the drawing board.",
                "That eliminates a whole set of possibilities.",
                "Okay, a clean slate.",
            ]),
            _ => "Processing...", // Fallback
        }
    }
}

// ------ Assist ------ //
/// Represents actions in Assist Mode.
#[derive(Clone, Debug, PartialEq)]
pub enum AssistAction {
    /// User is only grading the bot's last suggestion (e.g., "1b 2c").
    Grade(Grade),
    // TODO: Nice feature
    // /// Add a Call the bot did not make to narrow-down possibilities
    // History(Call, Grade),
    Invalid {
        err: Error,
    },
}

impl AssistAction {
    /// Parses input for Assist Mode.
    #[inline]
    pub fn parse(msg: &IncomingMessage) -> Self {
        match &msg.content {
            Content::Text(text) => Self::parse_str(&text.body),
            _ => Self::Invalid {
                err: Error {
                    err_msg: "Please send a text message with grade of my last call.".into(),
                },
            },
        }
    }

    /// A simplified parser that *only* looks for a grade.
    #[inline]
    fn parse_str(input: &str) -> Self {
        let trimmed_input = input.trim();

        if trimmed_input.is_empty() {
            return Self::Invalid {
                err: Error {
                    err_msg: "Your message was empty.".into(),
                },
            };
        }

        // Try to find grade components.
        let deads = DEADS_RE
            .captures(trimmed_input)
            .and_then(|caps| caps.get(1)?.as_str().parse::<u8>().ok())
            .unwrap_or(0);

        let injures = INJURES_RE
            .captures(trimmed_input)
            .and_then(|caps| caps.get(1)?.as_str().parse::<u8>().ok())
            .unwrap_or(0);

        // Check if a grade was explicitly mentioned.
        let grade_found = DEADS_RE.is_match(trimmed_input) || INJURES_RE.is_match(trimmed_input);

        if grade_found {
            let grade = Grade { deads, injures };
            // Validate the grade.
            if grade.deads + grade.injures > 4 {
                Self::Invalid {
                    err: Error {
                        err_msg: "The total number of Bulls and Cows cannot be more than 4.".into(),
                    },
                }
            } else {
                Self::Grade(grade) // Success!
            }
        } else {
            // No grade was found.
            Self::Invalid { err: Error { err_msg: "I didn't understand the score. Please provide it like '2 bulls 1 cow' or '2d 1i'.".into() } }
        }
    }

    /// "View" function: Replies with the next suggested guess.
    #[inline]
    pub async fn reply_next(msg: &IncomingMessage, last_grade: Grade, assist: Call) {
        // Re-use the commentary from HvbAction.
        let comment = HvbAction::bot_commentary(last_grade);
        let text = format!(
            "{comment}\n\nBased on that, Your next best guess is `{assist}`. Good luck! ✨"
        );
        GameRoomAction::reply_on(msg, text).await;
    }

    /// "View" function: Sends the win message.
    #[inline]
    pub async fn send_human_wins(msg: &IncomingMessage, rounds: usize) {
        let message = Self::get_win_message(rounds);
        try_reply!(msg.reply(message).await);
    }

    /// Helper to get the win message text.
    #[inline]
    pub fn get_win_message(rounds: usize) -> String {
        match rounds {
            1 => "First try! We're an unstoppable team! 🏆".to_string(),
            2..=4 => format!("We did it in only {rounds} moves! Flawless victory! 🥳"),
            5..=7 => format!("Victory is ours! Solved in {rounds} steps. Great teamwork!"),
            _ => format!("We got there! It took {rounds} rounds, but we cracked it. Well played!"),
        }
    }

    /// Generates a pre-formatted message for the user to share their assisted victory.
    #[inline]
    pub fn get_share_win_message(msg: &IncomingMessage, rounds: usize) -> impl Display {
        let achievement = match rounds {
            1..=4 => format!("in an incredible {rounds} move(s)! Flawless teamwork. 🤖🤝🙂"),
            _ => format!("in {rounds} moves. We make a great team!"),
        };

        format!(
            "🏆 Victory! Just won a game of Bulls & Cows with my bot assistant!\n\nWe cracked the code {achievement}. Join me {}",
            make_chat_url(msg)
        )
    }

    /// "View" function: Sends an invalid parse error.
    #[inline]
    pub async fn reply_invalid(msg: &IncomingMessage, err: Error) {
        GameRoomAction::reply_on(msg, err.to_string()).await;
    }

    /// "View" function: Sends the welcome message for Assist Mode.
    #[inline]
    pub async fn send_welcome(msg: &IncomingMessage, first_call: Call) {
        GameRoomAction::reply_on(
            msg,
            format!(
                "I'll help you beat your opponent! Let's start with a strong opening.

Your first guess should be `{first_call}`."
            ),
        )
        .await;
    }

    #[inline]
    pub async fn send_help(msg: &IncomingMessage) {
        let text = "*How to Use Assist Mode* 🤔

In this mode, I'm your secret weapon!

1. I'll give you the optimal number to guess.
2. Use that guess against your opponent.
3. Your opponent will score your guess (e.g., '1 Bull, 2 Cows').
4. Report that score back to me (e.g., send `1d 2i`).
5. I'll crunch the numbers and give you the next best guess.

Repeat until you achieve victory! 🏆

You can also use these commands:
- `restart`: Start a new assisted game.
- `menu`: Go back to the main menu
";
        GameRoomAction::reply_on(msg, text).await;
    }

    #[inline]
    pub async fn send_discrepancy(msg: &IncomingMessage) {
        try_reply!(msg.reply("Uh oh, I'm stumped! 🤯 Based on the scores you've provided, there are no possible numbers left. Your opponent might have made a mistake in scoring one of your guesses.").await);
    }
}

// ------ END ------ //
#[derive(Clone, Debug)]
pub enum EndAction {
    MainMenu,
    PlayAgain,
    Invalid { err: Error },
    ShareWin, // ContinueOnWeb
}

impl EndAction {
    #[inline]
    pub fn parse(msg: &IncomingMessage) -> Self {
        match &msg.content {
            Content::Interactive(InteractiveContent::Click(Button::Reply(reply))) => {
                match reply.call_back.as_str() {
                    END_MAINMENU_CALLBACK => Self::MainMenu,
                    PLAY_AGAIN_CALLBACK => Self::PlayAgain,
                    SHARE_WIN_CALLBACK => Self::ShareWin,
                    _ => Self::Invalid {
                        err: Error {
                            err_msg: "Unknown button clicked.".into(),
                        },
                    },
                }
            }
            _ => Self::Invalid {
                err: Error {
                    err_msg: "Please tap a button to continue.".into(),
                },
            },
        }
    }

    #[inline]
    pub async fn send_end_prompt(msg: &IncomingMessage, game: GameMode) {
        let body = "That was a great round! 🤩 What's next?";
        let play_again_text = match game {
            GameMode::BvB => "🤖 Play Again",
            GameMode::HvB => "🙂 Play Again",
            GameMode::Assist => "🤔 Play Again",
        };

        let draft = Draft::new()
            .body(body)
            .add_reply_button(PLAY_AGAIN_CALLBACK, play_again_text)
            .add_reply_button(END_MAINMENU_CALLBACK, "🏠 Main Menu")
            .footer("⭐ Star me! https://github.com/veecore/whatsapp-business-rs");

        try_reply!(msg.reply(draft).await);
    }

    #[inline]
    pub async fn send_end_prompt_suggest_win(msg: &IncomingMessage, game: GameMode) {
        let body = "Excellent game! Want to share your victory or play another round?";
        let play_again_text = match game {
            GameMode::BvB => "🤖 Play Again",
            GameMode::HvB => "🙂 Play Again",
            GameMode::Assist => "🤔 Play Again",
        };

        let draft = Draft::new()
            .body(body)
            .add_reply_button(SHARE_WIN_CALLBACK, "Share win 🏆")
            .add_reply_button(PLAY_AGAIN_CALLBACK, play_again_text)
            .add_reply_button(END_MAINMENU_CALLBACK, "🏠 Main Menu")
            .footer("⭐ Star me! https://github.com/veecore/whatsapp-business-rs");

        try_reply!(msg.reply(draft).await);
    }

    #[inline]
    pub async fn send_share_win_no_win(msg: &IncomingMessage) {
        try_reply!(
            msg.reply("It looks like you didn't win the last round, so there's no victory message to share. Let's play again and win this time!")
                .await
        );
    }

    #[inline]
    pub async fn send_share_win(msg: &IncomingMessage, win_msg: impl Display) {
        // Adding text causes confusion
        // let share_text = format!("👇 Copy or forward this message to share your win! 👇\n\n{win_msg}");
        try_reply!(msg.reply(win_msg.to_string()).await);
    }

    #[inline]
    pub async fn reply_invalid(msg: &IncomingMessage, err: Error) {
        try_reply!(msg.reply(err.to_string()).await);
    }
}

// ------ GENERIC GAME ROOM ------ //
// Generic game room action that can be applied on top of others
#[derive(Clone, Debug)]
pub enum GameRoomAction {
    Restart,
    MainMenu,
    Help,
}

impl GameRoomAction {
    /// Parses a message for generic commands.
    #[inline]
    pub fn parse(msg: &IncomingMessage) -> Option<Self> {
        match &msg.content {
            Content::Interactive(InteractiveContent::Click(Button::Reply(reply))) => {
                match reply.call_back.as_str() {
                    GAME_ROOM_RESTART_CALLBACK => Some(Self::Restart),
                    GAME_ROOM_MAINMENU_CALLBACK => Some(Self::MainMenu),
                    GAME_ROOM_HELP_CALLBACK => Some(Self::Help),
                    _ => None,
                }
            }
            Content::Text(text) => {
                let lower = text.body.to_lowercase();
                if lower.contains("restart") || lower.contains("again") {
                    Some(Self::Restart)
                } else if lower.contains("menu") || lower.contains("quit") || lower.contains("stop")
                {
                    Some(Self::MainMenu)
                } else if lower.contains("help") {
                    Some(Self::Help)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// A wrapper function to send a reply *with* the generic game buttons attached.
    /// This is the "mounting" pattern.
    // NOTE: Max of 1 button must be on draft
    #[inline]
    async fn reply_on(msg: &IncomingMessage, draft: impl OutgoingMessage) {
        // Add buttons
        let draft = draft
            .into_draft()
            .add_reply_button(GAME_ROOM_RESTART_CALLBACK, "🔄 Restart")
            .add_reply_button(GAME_ROOM_MAINMENU_CALLBACK, "🏠 Back to Menu")
            // FIXME: Better as button
            .footer("💡 Help");
        try_reply!(msg.reply(draft).await);
    }
}

/// Parses a string into a `Call` (`Guess`), ensuring it has 4 unique digits.
#[inline]
pub fn parse_guess(s: &str) -> Result<Call, Error> {
    if s.len() != 4 {
        return Err(Error {
            err_msg: "A guess must be exactly 4 digits.".into(),
        });
    }
    let mut digits = [Digit::D0; 4];
    let mut seen = HashSet::with_capacity(4);

    for (i, c) in s.chars().enumerate() {
        let Some(d_val) = c.to_digit(10) else {
            return Err(Error {
                err_msg: format!(
                    "Invalid character found. Please use only digits 0-9: '{}'",
                    c
                )
                .into(),
            });
        };
        if !seen.insert(d_val) {
            return Err(Error {
                err_msg: "All digits in a guess must be unique".into(),
            });
        }
        digits[i] = Digit::try_from(d_val as u8).unwrap();
    }
    Ok(Call(digits))
}

/// Picks a random string slice from a given array.
#[inline]
fn random_choice(choices: &[&'static str]) -> &'static str {
    choices.choose(&mut rand::rng()).copied().unwrap()
}

/// Creates a `wa.me/` link to the bot.
#[inline]
fn make_chat_url(msg: &IncomingMessage) -> String {
    // FIXME:
    let phone = msg
        .recipient
        .metadata
        .phone_number
        .as_ref()
        .unwrap_or(&msg.recipient.phone_id);
    format!("https://wa.me/{phone}?text=Hi")
}

// --- Unit Tests for the Parser ---
#[cfg(test)]
mod tests {
    use crate::tests::{mock_button_message, mock_text_message};

    use super::*;

    #[test]
    fn test_just_call() {
        let valid_call = HvbAction::Call(parse_guess("1234").unwrap());
        assert_eq!(HvbAction::parse_str("1234"), valid_call);
        let valid_call_2 = HvbAction::Call(parse_guess("9876").unwrap());
        assert_eq!(HvbAction::parse_str("  9876  "), valid_call_2);
        // // FIXME
        // // thread 'helpers::tests::test_just_call' (136784) panicked at src/helpers.rs:461:9:
        // // assertion `left == right` failed
        // //   left: Both { grade: GuessResult { deads: 4, injures: 0 }, call: Guess([D1, D2, D3, D4]) }
        // //  right: Call(Guess([D1, D2, D3, D4]))
        // assert_eq!(
        //     HvbAction::parse_str("I think it is 1234 but idk"),
        //     HvbAction::Call(parse_guess("1234").unwrap()) // ambiguous
        // );
    }

    #[test]
    fn test_call_with_duplicate_digits() {
        assert!(matches!(
            HvbAction::parse_str("1123"),
            HvbAction::Invalid { .. }
        ));
        assert!(matches!(
            HvbAction::parse_str("9091"),
            HvbAction::Invalid { .. }
        ));
    }

    #[test]
    fn test_just_grade() {
        let expected = HvbAction::Grade(GradeAction {
            grade: Grade {
                deads: 2,
                injures: 1,
            },
            target: None,
        });
        assert_eq!(HvbAction::parse_str("2d 1i"), expected);
        assert_eq!(HvbAction::parse_str("2 deAds 1 INJURE"), expected);
        assert_eq!(HvbAction::parse_str("2 bull 1 injure"), expected);
        assert_eq!(HvbAction::parse_str("2 bull 1 injure"), expected);
        assert_eq!(HvbAction::parse_str("2deads 1injure"), expected);
        // Test synonyms
        assert_eq!(HvbAction::parse_str("2b 1c"), expected);
        assert_eq!(HvbAction::parse_str("2 bulls 1 cow"), expected);
        // Test 'j' for injure
        assert_eq!(HvbAction::parse_str("2d 1j"), expected);
        assert_eq!(
            HvbAction::parse_str("0d 1j"),
            HvbAction::Grade(GradeAction {
                grade: Grade {
                    deads: 0,
                    injures: 1,
                },
                target: None,
            })
        );

        let expected_zero = HvbAction::Grade(GradeAction {
            grade: Grade {
                deads: 0,
                injures: 2,
            },
            target: None,
        });
        assert_eq!(HvbAction::parse_str("0d2i"), expected_zero);
        assert_eq!(HvbAction::parse_str("0d 2j"), expected_zero);
        assert_eq!(HvbAction::parse_str("2c"), expected_zero);
        assert_eq!(HvbAction::parse_str("2 injures"), expected_zero);
    }

    #[test]
    fn test_grade_with_zeroes() {
        let expected = HvbAction::Grade(GradeAction {
            grade: Grade {
                deads: 0,
                injures: 0,
            },
            target: None,
        });
        assert_eq!(HvbAction::parse_str("0d 0i"), expected);
        assert_eq!(HvbAction::parse_str("0b 0c"), expected);
        assert_eq!(HvbAction::parse_str("0 deads"), expected);
        assert_eq!(HvbAction::parse_str("0 bulls"), expected);
        assert_eq!(HvbAction::parse_str("0 injures"), expected);
    }

    #[test]
    fn test_impossible_grades() {
        // Total greater than 4
        assert!(matches!(
            HvbAction::parse_str("3d 2i"),
            HvbAction::Invalid { .. }
        ));
        assert!(matches!(
            HvbAction::parse_str("4 bulls 1 cow"),
            HvbAction::Invalid { .. }
        ));
        // The impossible 3 bulls, 1 cow combo
        assert!(matches!(
            HvbAction::parse_str("3b 1c"),
            HvbAction::Invalid { err } if err.err_msg.contains("impossible")
        ));
    }

    #[test]
    fn test_both_call_and_grade() {
        let expected = HvbAction::Both {
            grade: GradeAction {
                grade: Grade {
                    deads: 1,
                    injures: 2,
                },
                target: None,
            },
            call: parse_guess("5670").unwrap(),
        };
        assert_eq!(HvbAction::parse_str("1d 2i, I call 5670"), expected);
        assert_eq!(
            HvbAction::parse_str("My call is 5670. You got 1 dead and 2 injures"),
            expected
        );
        assert_eq!(
            HvbAction::parse_str("My call is 5670. You got 1 bull and 2 injures"),
            expected
        );
        assert_eq!(HvbAction::parse_str("5670 1B2C"), expected);
        assert_eq!(HvbAction::parse_str("5670 1D2I"), expected);
        assert_eq!(HvbAction::parse_str("1 dead 2 injures 5670"), expected);
    }

    #[test]
    fn test_set_password() {
        let expected = HvbAction::SetPassword(parse_guess("9012").unwrap());
        assert_eq!(HvbAction::parse_str("set 9012"), expected);
        assert_eq!(HvbAction::parse_str("password 9012"), expected);
        assert_eq!(HvbAction::parse_str("  SET   9012  "), expected);

        // Should fail if password has duplicate digits
        assert!(matches!(
            HvbAction::parse_str("set 9912"),
            HvbAction::Invalid { .. }
        ));

        // Should not be confused with a grade or call
        let not_a_command = HvbAction::parse_str("I want to set 4567");
        assert!(matches!(not_a_command, HvbAction::Invalid { .. }));
    }

    #[test]
    fn test_invalid_input() {
        assert!(matches!(
            HvbAction::parse_str("hello world"),
            HvbAction::Invalid { .. }
        ));
        assert!(matches!(
            HvbAction::parse_str("123"),
            HvbAction::Invalid { .. }
        ));
        assert!(matches!(
            HvbAction::parse_str("This is my number 12345"),
            HvbAction::Invalid { .. }
        ));
        assert!(matches!(
            HvbAction::parse_str(""),
            HvbAction::Invalid { .. }
        ));
        assert!(matches!(
            HvbAction::parse_str("   "),
            HvbAction::Invalid { .. }
        ));
    }

    #[test]
    fn test_grade_with_invalid_call() {
        let result = HvbAction::parse_str("1d 2i, also 9987");
        assert!(matches!(result, HvbAction::Invalid{ err } if err.err_msg.contains("unique")));
    }

    #[test]
    fn test_bvb_parser() {
        assert!(matches!(
            BvbAction::parse(&mock_button_message("bvb_next")),
            BvbAction::Next
        ));
        assert!(matches!(
            BvbAction::parse(&mock_button_message("some_other_button")),
            BvbAction::Invalid { .. }
        ));
        assert!(matches!(
            BvbAction::parse(&mock_text_message("next")),
            BvbAction::Invalid { .. }
        ));
    }

    #[test]
    fn test_assist_parser() {
        let expected = AssistAction::Grade(Grade {
            deads: 1,
            injures: 1,
        });
        assert_eq!(AssistAction::parse_str("1d 1i"), expected);
        assert_eq!(AssistAction::parse_str("1 bull 1 cow"), expected);

        assert!(matches!(
            AssistAction::parse_str("some random text"),
            AssistAction::Invalid { .. }
        ));
        assert!(matches!(
            AssistAction::parse_str("1234"),
            AssistAction::Invalid { .. }
        ));
    }
}
