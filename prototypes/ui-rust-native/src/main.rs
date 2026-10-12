// UI Rust-native prototype — Phase 0 benchmark (ISSUE-009)
// Minimal: open file, edit buffer, render preview.

use iced::widget::{column, container, row, scrollable, text_editor, text};
use iced::{Element, Length, Sandbox, Settings};

#[derive(Debug, Clone)]
enum Message {
    EditorAction(text_editor::Action),
}

struct UiRustNative {
    editor: text_editor::Content,
}

impl Sandbox for UiRustNative {
    type Message = Message;

    fn new() -> Self {
        Self {
            editor: text_editor::Content::new(),
        }
    }

    fn title(&self) -> String {
        String::from("ui-rust-native prototype")
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::EditorAction(action) => {
                let start = std::time::Instant::now();
                self.editor.perform(action);
                let duration = start.elapsed();
                eprintln!("Input latency: {:?}", duration);
            }
        }
    }

    fn view(&self) -> Element<Message> {
        let editor = text_editor(&self.editor)
            .on_action(Message::EditorAction);

        let preview = container(
            text(self.editor.text())
                .size(14)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(10);

        row![
            scrollable(editor).width(Length::FillPortion(50)),
            scrollable(preview).width(Length::FillPortion(50)),
        ]
        .into()
    }
}

fn main() -> iced::Result {
    UiRustNative::run(Settings::default())
}
