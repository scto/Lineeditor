use std::io::stdout;
use std::io::Result;

use clipboard::ClipboardContext;
use clipboard::ClipboardProvider;
use crossterm::cursor::position;
use crossterm::cursor::SetCursorStyle;
use crossterm::event;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableFocusChange;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::EnableFocusChange;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::PopKeyboardEnhancementFlags;
use crossterm::event::PushKeyboardEnhancementFlags;
use crossterm::execute;
use crossterm::terminal;

use crate::completion::Suggestion;
use crate::editor::Editor;
use crate::event::EditCommand;
use crate::event::LineEditorEvent;
use crate::event::MovementCommand;
use crate::input_filter::filter_input;
use crate::input_filter::InputFilter;
use crate::keybindings::KeyCombination;
use crate::keybindings::Keybindings;
use crate::style::Style;
use crate::styled_editor_view::StyledEditorView;
use crate::AutoPair;
use crate::Completer;
use crate::DropDownListView;
use crate::Highlighter;
use crate::Hinter;
use crate::ListView;
use crate::Prompt;
use crate::DEFAULT_PAIRS;

/// A Result can return from`LineEditor::read_line()`
#[derive(Debug)]
pub enum LineEditorResult {
    /// Entry succeeded with the provided content
    Success(String),
    /// Interrupt current editing
    Interrupted,
    /// End terminal session
    EndTerminalSession,
}

/// An internal Status returned after applying event
enum EventStatus {
    /// General Event Handled
    GeneralHandled,
    /// Edit Event is handled
    EditHandled,
    /// Movement Event is handled
    MovementHandled,
    /// Selection Event is handled
    SelectionHandled,
    /// Auto Complete Event is handled
    AutoCompleteHandled,
    /// Event is in applicable to handle
    Inapplicable,
    /// Exit with Result or Error
    Exits(LineEditorResult),
}

/// Line Editor Engine
pub struct LineEditor {
    prompt: Box<dyn Prompt>,
    editor: Editor,
    input_filter: InputFilter,
    styled_editor_text: StyledEditorView,
    keybindings: Keybindings,
    auto_pair: Option<Box<dyn AutoPair>>,
    highlighters: Vec<Box<dyn Highlighter>>,
    hinters: Vec<Box<dyn Hinter>>,

    completer: Option<Box<dyn Completer>>,
    auto_complete_view: Box<dyn ListView<Suggestion>>,

    cursor_style: Option<SetCursorStyle>,
    selection_style: Option<Style>,
    selected_start: u16,
    selected_end: u16,
    enable_surround_selection: bool,
}

impl LineEditor {
    /// Create new instance of LineEditor with Prompt
    #[must_use]
    pub fn new(prompt: Box<dyn Prompt>) -> Self {
        LineEditor {
            prompt,
            editor: Editor::default(),
            input_filter: InputFilter::Text,
            styled_editor_text: StyledEditorView::default(),
            keybindings: Keybindings::default(),
            auto_pair: None,
            highlighters: vec![],
            hinters: vec![],
            completer: None,
            auto_complete_view: Box::<DropDownListView>::default(),
            cursor_style: None,

            selection_style: None,
            selected_start: 0,
            selected_end: 0,
            enable_surround_selection: false,
        }
    }

    /// Wait for input and provide the user
    ///
    /// Returns a [`std::io::Result`] in which the `Err` type is [`std::io::Result`]
    /// and the `Ok` variant wraps a [`LineEditorResult`] which handles user inputs.
    pub fn read_line(&mut self) -> Result<LineEditorResult> {
        if let Some(cursor_style) = self.cursor_style {
            self.styled_editor_text.set_cursor_style(cursor_style)?;
        }

        terminal::enable_raw_mode()?;
        execute!(
            stdout(),
            EnableBracketedPaste,
            EnableFocusChange,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;

        let result = self.read_line_helper();

        terminal::disable_raw_mode()?;
        execute!(
            stdout(),
            DisableBracketedPaste,
            PopKeyboardEnhancementFlags,
            DisableFocusChange,
            DisableMouseCapture
        )?;

        let default_cursor_style = SetCursorStyle::DefaultUserShape;
        self.styled_editor_text
            .set_cursor_style(default_cursor_style)?;
        result
    }

    /// Set style for visual selection or NONE to clear it
    pub fn set_visual_selection_style(&mut self, style: Option<Style>) {
        self.selection_style = style;
    }

    /// Get the current Editor
    pub fn editor(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// Get the current Keybindings
    pub fn keybinding(&mut self) -> &mut Keybindings {
        &mut self.keybindings
    }

    /// Set the current InputFilter type
    pub fn set_input_filter(&mut self, input_filter: InputFilter) {
        self.input_filter = input_filter;
    }

    /// Add Auto pair, or clear it by passing None
    pub fn set_auto_pair(&mut self, auto_pair: Option<Box<dyn AutoPair>>) {
        self.auto_pair = auto_pair
    }

    /// Set the current cursor style
    /// Or `None` to reset
    pub fn set_cursor_style(&mut self, style: Option<SetCursorStyle>) {
        self.cursor_style = style;
    }

    /// Get the current list of highlighters
    pub fn highlighters(&mut self) -> &mut Vec<Box<dyn Highlighter>> {
        &mut self.highlighters
    }

    /// Add new Syntax highlighter
    pub fn add_highlighter(&mut self, highlighter: Box<dyn Highlighter>) {
        self.highlighters.push(highlighter);
    }

    /// Clear current syntax highlighter
    pub fn clear_highlighters(&mut self) {
        self.highlighters.clear();
    }

    /// Get current hinters
    pub fn hinters(&mut self) -> &mut Vec<Box<dyn Hinter>> {
        &mut self.hinters
    }

    /// Add new Hinter
    pub fn add_hinter(&mut self, hinter: Box<dyn Hinter>) {
        self.hinters.push(hinter);
    }

    /// Clear current hinters
    pub fn clear_hinters(&mut self) {
        self.hinters.clear();
    }

    /// Set the current Auto completer
    pub fn set_completer(&mut self, completer: Box<dyn Completer>) {
        self.completer = Some(completer);
    }

    /// Clear current auto completer
    pub fn clear_completer(&mut self) {
        self.completer = None
    }

    /// Set the current Auto Complete View
    pub fn set_auto_complete_view(&mut self, auto_complete_view: Box<dyn ListView<Suggestion>>) {
        self.auto_complete_view = auto_complete_view;
    }

    /// Enable or Disable surround selection feature
    pub fn enable_surround_selection(&mut self, enable: bool) {
        self.enable_surround_selection = enable;
    }

    /// Helper implementing the logic for [`LineEditor::read_line()`] to be wrapped
    /// in a `raw_mode` context.
    fn read_line_helper(&mut self) -> Result<LineEditorResult> {
        let mut lineeditor_events: Vec<LineEditorEvent> = vec![];

        let prompt_buffer = self.prompt.prompt();
        let prompt_len = prompt_buffer.len() as u16;

        let row_start = position().unwrap().1;
        self.styled_editor_text
            .set_start_position((prompt_len, row_start));
        self.styled_editor_text
            .render_prompt_buffer(&prompt_buffer)?;

        'main: loop {
            loop {
                match event::read()? {
                    Event::Key(key_event) => match key_event.code {
                        KeyCode::Char(ch) => {
                            if (key_event.modifiers == KeyModifiers::NONE
                                || key_event.modifiers == KeyModifiers::SHIFT)
                                && key_event.kind == KeyEventKind::Press
                            {
                                if filter_input(ch, &self.input_filter) {
                                    let commands = vec![EditCommand::InsertChar(ch)];
                                    let edit_command = LineEditorEvent::Edit(commands);
                                    lineeditor_events.push(edit_command);
                                }
                                break;
                            }

                            let key_combination = KeyCombination::from(key_event);
                            if let Some(command) = self.keybindings.find_binding(key_combination) {
                                lineeditor_events.push(command);
                                break;
                            }
                        }
                        _ => {
                            let key_combination = KeyCombination::from(key_event);
                            if let Some(command) = self.keybindings.find_binding(key_combination) {
                                lineeditor_events.push(command);
                                break;
                            }
                        }
                    },
                    Event::Paste(string) => {
                        lineeditor_events.push(LineEditorEvent::Edit(vec![
                            EditCommand::InsertString(string),
                        ]));
                        break;
                    }
                    _ => {}
                }
            }

            // Track the buffer size at the start
            let buffer_len_before = self.editor.styled_buffer().len();

            // Apply the list of events
            for event in lineeditor_events.drain(..) {
                match self.handle_editor_event(&event)? {
                    EventStatus::AutoCompleteHandled => {
                        continue 'main;
                    }
                    EventStatus::Inapplicable => {
                        continue 'main;
                    }
                    EventStatus::Exits(result) => return Ok(result),
                    _ => {}
                }
            }

            // Run the auto pair complete if one char is inserted
            if buffer_len_before < self.editor.styled_buffer().len() {
                // Auto pair complete
                if let Some(auto_pair) = &self.auto_pair {
                    auto_pair.complete_pair(self.editor.styled_buffer());
                }
            }

            // Reset styled buffer styles
            self.editor.styled_buffer().reset_styles();

            // Apply all registered syntax highlighter in insertion order
            for highlighter in &self.highlighters {
                highlighter.highlight(self.editor.styled_buffer());
            }

            // Apply visual selection
            self.apply_visual_selection();

            // Render the current buffer with style
            self.styled_editor_text
                .render_line_buffer(self.editor.styled_buffer())?;

            // If cursor is at the end of the buffer, check if hint is available
            if self.editor.styled_buffer().position() == self.editor.styled_buffer().len() {
                for hinter in &self.hinters {
                    if let Some(hint) = hinter.hint(self.editor.styled_buffer()) {
                        self.styled_editor_text.render_hint(&hint)?;
                        break;
                    }
                }
            }
        }
    }

    /// Apply LineEditorEvent and return handling status
    fn handle_editor_event(&mut self, event: &LineEditorEvent) -> Result<EventStatus> {
        match event {
            LineEditorEvent::Edit(commands) => {
                for command in commands {
                    if self.enable_surround_selection && self.selected_start != self.selected_end {
                        if let EditCommand::InsertChar(c) = &command {
                            for (key, value) in DEFAULT_PAIRS {
                                if key == c {
                                    self.apply_surround_selection(*key, *value);
                                    return Ok(EventStatus::EditHandled);
                                }
                            }
                        }
                    }
                    self.editor.run_edit_commands(command);
                }
                self.reset_selection_range();
                Ok(EventStatus::EditHandled)
            }
            LineEditorEvent::Movement(commands) => {
                for command in commands {
                    self.editor.run_movement_commands(command);
                }
                self.reset_selection_range();
                Ok(EventStatus::MovementHandled)
            }
            LineEditorEvent::Enter => {
                if self.auto_complete_view.is_visible() {
                    if let Some(suggestion) = self.auto_complete_view.selected_element() {
                        let literal = &suggestion.content.literal();
                        let span = &suggestion.span;

                        let delete_command = EditCommand::DeleteSpan(span.start, span.end);
                        self.editor.run_edit_commands(&delete_command);

                        let insert_command = EditCommand::InsertString(literal.to_string());
                        self.editor.run_edit_commands(&insert_command);

                        self.auto_complete_view.clear()?;
                        self.auto_complete_view.set_visibility(false);
                        return Ok(EventStatus::SelectionHandled);
                    }
                }

                let buffer = self.editor.styled_buffer().buffer().iter().collect();
                self.reset_selection_range();

                self.editor.styled_buffer().clear();

                Ok(EventStatus::Exits(LineEditorResult::Success(buffer)))
            }
            LineEditorEvent::Up => {
                if self.auto_complete_view.is_visible() {
                    self.auto_complete_view.focus_previous();
                    self.auto_complete_view.render()?;
                    return Ok(EventStatus::AutoCompleteHandled);
                }
                Ok(EventStatus::Inapplicable)
            }
            LineEditorEvent::Down => {
                if self.auto_complete_view.is_visible() {
                    self.auto_complete_view.focus_next();
                    self.auto_complete_view.clear()?;
                    self.auto_complete_view.render()?;
                    return Ok(EventStatus::AutoCompleteHandled);
                }
                Ok(EventStatus::Inapplicable)
            }
            LineEditorEvent::Left => {
                self.editor
                    .run_movement_commands(&MovementCommand::MoveLeftChar);
                self.reset_selection_range();
                Ok(EventStatus::MovementHandled)
            }
            LineEditorEvent::Right => {
                self.editor
                    .run_movement_commands(&MovementCommand::MoveRightChar);
                self.reset_selection_range();
                Ok(EventStatus::MovementHandled)
            }
            LineEditorEvent::Delete => {
                if self.selected_start != self.selected_end {
                    self.delete_selected_text();
                } else {
                    self.editor.run_edit_commands(&EditCommand::DeleteRightChar)
                }
                Ok(EventStatus::EditHandled)
            }
            LineEditorEvent::Backspace => {
                if self.selected_start != self.selected_end {
                    self.delete_selected_text();
                } else {
                    self.editor.run_edit_commands(&EditCommand::DeleteLeftChar)
                }
                Ok(EventStatus::EditHandled)
            }
            LineEditorEvent::SelectLeft => {
                if self.selected_end < 1 {
                    Ok(EventStatus::Inapplicable)
                } else {
                    self.selected_end -= 1;
                    Ok(EventStatus::SelectionHandled)
                }
            }
            LineEditorEvent::SelectRight => {
                if self.selected_end as usize > self.editor.styled_buffer().len() {
                    Ok(EventStatus::Inapplicable)
                } else {
                    self.selected_end += 1;
                    Ok(EventStatus::SelectionHandled)
                }
            }
            LineEditorEvent::SelectAll => {
                self.selected_start = 0;
                self.selected_end = self.editor.styled_buffer().len() as u16;
                Ok(EventStatus::SelectionHandled)
            }
            LineEditorEvent::CutSelected => {
                if self.selected_start != self.selected_end {
                    let from = usize::min(self.selected_start.into(), self.selected_end.into());
                    let to = usize::max(self.selected_start.into(), self.selected_end.into());
                    let styled_buffer = self.editor.styled_buffer();
                    if let Some(selected_text) = styled_buffer.sub_string(from, to) {
                        let mut clipboard_context: ClipboardContext =
                            ClipboardProvider::new().unwrap();
                        let _ = clipboard_context.set_contents(selected_text);

                        styled_buffer.delete_range(from, to);
                        self.reset_selection_range();
                        return Ok(EventStatus::GeneralHandled);
                    }
                }
                Ok(EventStatus::Inapplicable)
            }
            LineEditorEvent::CopySelected => {
                if self.selected_start != self.selected_end {
                    let from = usize::min(self.selected_start.into(), self.selected_end.into());
                    let to = usize::max(self.selected_start.into(), self.selected_end.into());
                    let styled_buffer = self.editor.styled_buffer();
                    if let Some(selected_text) = styled_buffer.sub_string(from, to) {
                        let mut clipboard_context: ClipboardContext =
                            ClipboardProvider::new().unwrap();
                        let _ = clipboard_context.set_contents(selected_text);
                        return Ok(EventStatus::GeneralHandled);
                    }
                }
                Ok(EventStatus::Inapplicable)
            }
            LineEditorEvent::Paste => {
                let mut clipboard_context: ClipboardContext = ClipboardProvider::new().unwrap();
                let clipboard_contents = clipboard_context.get_contents();
                if let Ok(content) = clipboard_contents {
                    if self.selected_start != self.selected_end {
                        self.delete_selected_text();
                    }

                    self.editor
                        .run_edit_commands(&EditCommand::InsertString(content));
                    return Ok(EventStatus::GeneralHandled);
                }
                Ok(EventStatus::Inapplicable)
            }
            LineEditorEvent::ToggleAutoComplete => {
                if self.auto_complete_view.is_visible() {
                    self.auto_complete_view.clear()?;
                    self.auto_complete_view.set_visibility(false);
                    return Ok(EventStatus::Inapplicable);
                }

                if let Some(completer) = &self.completer {
                    let mut suggestions = completer.complete(self.editor.styled_buffer());
                    if !suggestions.is_empty() {
                        let prompt_width = self.prompt.prompt().len() as u16;
                        let (_, row) = position()?;

                        let mut style = Style::default();
                        style.set_background_color(crossterm::style::Color::Blue);
                        self.auto_complete_view.set_focus_style(style);

                        self.auto_complete_view.reset();
                        self.auto_complete_view.set_elements(&mut suggestions);
                        self.auto_complete_view.clear()?;
                        self.auto_complete_view.render()?;
                        self.auto_complete_view.set_visibility(true);

                        let auto_complete_height = self.auto_complete_view.len();
                        let (_, max_row) = terminal::size()?;

                        if row + auto_complete_height as u16 > max_row {
                            let new_start_row = max_row - 2 - self.auto_complete_view.len() as u16;
                            self.styled_editor_text
                                .set_start_position((prompt_width, new_start_row));
                        }

                        return Ok(EventStatus::AutoCompleteHandled);
                    }

                    return Ok(EventStatus::Inapplicable);
                }

                Ok(EventStatus::Inapplicable)
            }
            _ => Ok(EventStatus::Inapplicable),
        }
    }

    /// Apply visual selection on the current styled buffer
    fn apply_visual_selection(&mut self) {
        if self.selected_start == self.selected_end {
            return;
        }

        // Apply visual selection style if it not None
        if let Some(style) = &self.selection_style {
            let styled_buffer = self.editor.styled_buffer();
            // Handle From and To, so we allow select from any direction
            let from = usize::min(self.selected_start.into(), self.selected_end.into());
            let to = usize::max(self.selected_start.into(), self.selected_end.into());
            styled_buffer.style_range(from, to, style.clone());
        }
    }

    /// Apply surround selection on the current styled buffer
    fn apply_surround_selection(&mut self, start: char, end: char) {
        let from = usize::min(self.selected_start.into(), self.selected_end.into());
        let to = usize::max(self.selected_start.into(), self.selected_end.into());

        let editor = self.editor.styled_buffer();
        editor.set_position(from);
        editor.insert_char(start);
        editor.set_position(to + 1);
        editor.insert_char(end);
        editor.set_position(from);
    }

    /// Delete the current selected text
    fn delete_selected_text(&mut self) {
        if self.selected_start == self.selected_end {
            return;
        }

        let from = usize::min(self.selected_start.into(), self.selected_end.into());
        let to = usize::max(self.selected_start.into(), self.selected_end.into());
        let delete_selection = EditCommand::DeleteSpan(from, to);
        self.editor.run_edit_commands(&delete_selection);
        self.editor.styled_buffer().set_position(from);
        self.reset_selection_range();
    }

    /// Reset selection start and end to be the current cursor position
    fn reset_selection_range(&mut self) {
        let position = self.editor.styled_buffer().position() as u16;
        self.selected_start = position;
        self.selected_end = position;
    }
}
