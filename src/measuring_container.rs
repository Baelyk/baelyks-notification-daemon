use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::widget;
use iced::advanced::Layout;
use iced::advanced::Shell;
use iced::Rectangle;
use iced::{Element, Length, Size};

#[derive(Default)]
struct State {
    size: Size,
}

pub struct MeasuringContainer<'a, Message, Theme, Renderer> {
    child: Element<'a, Message, Theme, Renderer>,
    on_resize: Box<dyn Fn(Size) -> Message + 'a>,
}

impl<'a, Message, Theme, Renderer> MeasuringContainer<'a, Message, Theme, Renderer>
where
    Theme: iced::widget::text::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer + 'a,
{
    pub fn new<F>(child: Element<'a, Message, Theme, Renderer>, on_resize: F) -> Self
    where
        F: Fn(Size) -> Message + 'a,
    {
        Self {
            child,
            on_resize: Box::new(on_resize),
        }
    }
}

impl<'a, Message, Theme, Renderer> widget::Widget<Message, Theme, Renderer>
    for MeasuringContainer<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // On redraws, measure the size of the widget, and if it's different from last time,
        // publish a message with the new size
        if let iced::Event::Window(iced::window::Event::RedrawRequested(_)) = event {
            let limits = layout::Limits::new(Size::ZERO, Size::INFINITY);
            let new_size = self.layout(tree, renderer, &limits).bounds().size();
            let state = tree.state.downcast_mut::<State>();

            if new_size != state.size {
                state.size = new_size;
                shell.publish((self.on_resize)(new_size));
            }
        }

        // Let the contents capture the event
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Shrink,
            height: Length::Shrink,
        }
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(&self.child)]
    }

    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(&[&self.child]);
    }

    fn layout(
        &self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let contents = self
            .child
            .as_widget()
            .layout(&mut tree.children[0], renderer, limits);

        layout::Node::container(contents, 0.into())
    }

    fn draw(
        &self,
        state: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let contents_layout = layout.children().next().unwrap();

        self.child.as_widget().draw(
            &state.children[0],
            renderer,
            theme,
            style,
            contents_layout,
            cursor,
            viewport,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<MeasuringContainer<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(widget: MeasuringContainer<'a, Message, Theme, Renderer>) -> Self {
        Self::new(widget)
    }
}
