use crate::{
    buffer::Buffer,
    layout::{Position, Rect},
    widgets::{StatefulWidget, StatefulWidgetRef, Widget, WidgetRef},
};

#[cfg(feature = "native-scrolling")]
use crate::buffer::Cell;

/// Captured snapshot of buffer content for native scrollback.
///
/// This is created when [`Frame::set_scroll_up`] is called, capturing the current
/// buffer state at that moment. This allows overlays (like modals) to be rendered
/// after calling `set_scroll_up` without affecting what goes to scrollback.
#[cfg(feature = "native-scrolling")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScrollSnapshot {
    /// Number of lines to scroll into native scrollback.
    pub(crate) lines: u16,
    /// Captured cells from the top `lines` rows of the buffer at the time
    /// `set_scroll_up` was called. Length is `lines * width`.
    pub(crate) content: Vec<Cell>,
    /// Width of each row in the snapshot.
    pub(crate) width: u16,
}

/// A consistent view into the terminal state for rendering a single frame.
///
/// This is obtained via the closure argument of [`Terminal::draw`]. It is used to render widgets
/// to the terminal and control the cursor position.
///
/// The changes drawn to the frame are applied only to the current [`Buffer`]. After the closure
/// returns, the current buffer is compared to the previous buffer and only the changes are applied
/// to the terminal. This avoids drawing redundant cells.
///
/// [`Buffer`]: crate::buffer::Buffer
/// [`Terminal::draw`]: crate::Terminal::draw
#[derive(Debug, Hash)]
pub struct Frame<'a> {
    /// Where should the cursor be after drawing this frame?
    ///
    /// If `None`, the cursor is hidden and its position is controlled by the backend. If `Some((x,
    /// y))`, the cursor is shown and placed at `(x, y)` after the call to `Terminal::draw()`.
    pub(crate) cursor_position: Option<Position>,

    /// The area of the viewport
    pub(crate) viewport_area: Rect,

    /// The buffer that is used to draw the current frame
    pub(crate) buffer: &'a mut Buffer,

    /// The frame count indicating the sequence number of this frame.
    pub(crate) count: usize,

    /// Captured scroll snapshot for native scrollback support.
    ///
    /// When [`set_scroll_up`](Frame::set_scroll_up) is called, the current buffer state
    /// is captured into this snapshot. This allows the application to continue rendering
    /// (e.g., adding modals) without affecting what goes to the native scrollback buffer.
    ///
    /// This field is only available when the `native-scrolling` feature is enabled.
    #[cfg(feature = "native-scrolling")]
    pub(crate) scroll_snapshot: Option<ScrollSnapshot>,
}

/// `CompletedFrame` represents the state of the terminal after all changes performed in the last
/// [`Terminal::draw`] call have been applied. Therefore, it is only valid until the next call to
/// [`Terminal::draw`].
///
/// [`Terminal::draw`]: crate::Terminal::draw
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CompletedFrame<'a> {
    /// The buffer that was used to draw the last frame.
    pub buffer: &'a Buffer,
    /// The size of the last frame.
    pub area: Rect,
    /// The frame count indicating the sequence number of this frame.
    pub count: usize,
}

impl Frame<'_> {
    /// The area of the current frame
    ///
    /// This is guaranteed not to change during rendering, so may be called multiple times.
    ///
    /// If your app listens for a resize event from the backend, it should ignore the values from
    /// the event for any calculations that are used to render the current frame and use this value
    /// instead as this is the area of the buffer that is used to render the current frame.
    pub const fn area(&self) -> Rect {
        self.viewport_area
    }

    /// The area of the current frame
    ///
    /// This is guaranteed not to change during rendering, so may be called multiple times.
    ///
    /// If your app listens for a resize event from the backend, it should ignore the values from
    /// the event for any calculations that are used to render the current frame and use this value
    /// instead as this is the area of the buffer that is used to render the current frame.
    #[deprecated = "use .area() as it's the more correct name"]
    pub const fn size(&self) -> Rect {
        self.viewport_area
    }

    /// Render a [`Widget`] to the current buffer using [`Widget::render`].
    ///
    /// Usually the area argument is the size of the current frame or a sub-area of the current
    /// frame (which can be obtained using [`Layout`] to split the total area).
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ratatui::{backend::TestBackend, Terminal};
    /// # let backend = TestBackend::new(5, 5);
    /// # let mut terminal = Terminal::new(backend).unwrap();
    /// # let mut frame = terminal.get_frame();
    /// use ratatui::{layout::Rect, widgets::Block};
    ///
    /// let block = Block::new();
    /// let area = Rect::new(0, 0, 5, 5);
    /// frame.render_widget(block, area);
    /// ```
    ///
    /// [`Layout`]: crate::layout::Layout
    pub fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buffer);
    }

    /// Render a [`WidgetRef`] to the current buffer using [`WidgetRef::render_ref`].
    ///
    /// Usually the area argument is the size of the current frame or a sub-area of the current
    /// frame (which can be obtained using [`Layout`] to split the total area).
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[cfg(feature = "unstable-widget-ref")] {
    /// # use ratatui::{backend::TestBackend, Terminal};
    /// # let backend = TestBackend::new(5, 5);
    /// # let mut terminal = Terminal::new(backend).unwrap();
    /// # let mut frame = terminal.get_frame();
    /// use ratatui::{layout::Rect, widgets::Block};
    ///
    /// let block = Block::new();
    /// let area = Rect::new(0, 0, 5, 5);
    /// frame.render_widget_ref(block, area);
    /// # }
    /// ```
    #[allow(clippy::needless_pass_by_value)]
    #[instability::unstable(feature = "widget-ref")]
    pub fn render_widget_ref<W: WidgetRef>(&mut self, widget: W, area: Rect) {
        widget.render_ref(area, self.buffer);
    }

    /// Render a [`StatefulWidget`] to the current buffer using [`StatefulWidget::render`].
    ///
    /// Usually the area argument is the size of the current frame or a sub-area of the current
    /// frame (which can be obtained using [`Layout`] to split the total area).
    ///
    /// The last argument should be an instance of the [`StatefulWidget::State`] associated to the
    /// given [`StatefulWidget`].
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ratatui::{backend::TestBackend, Terminal};
    /// # let backend = TestBackend::new(5, 5);
    /// # let mut terminal = Terminal::new(backend).unwrap();
    /// # let mut frame = terminal.get_frame();
    /// use ratatui::{
    ///     layout::Rect,
    ///     widgets::{List, ListItem, ListState},
    /// };
    ///
    /// let mut state = ListState::default().with_selected(Some(1));
    /// let list = List::new(vec![ListItem::new("Item 1"), ListItem::new("Item 2")]);
    /// let area = Rect::new(0, 0, 5, 5);
    /// frame.render_stateful_widget(list, area, &mut state);
    /// ```
    ///
    /// [`Layout`]: crate::layout::Layout
    pub fn render_stateful_widget<W>(&mut self, widget: W, area: Rect, state: &mut W::State)
    where
        W: StatefulWidget,
    {
        widget.render(area, self.buffer, state);
    }

    /// Render a [`StatefulWidgetRef`] to the current buffer using
    /// [`StatefulWidgetRef::render_ref`].
    ///
    /// Usually the area argument is the size of the current frame or a sub-area of the current
    /// frame (which can be obtained using [`Layout`] to split the total area).
    ///
    /// The last argument should be an instance of the [`StatefulWidgetRef::State`] associated to
    /// the given [`StatefulWidgetRef`].
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[cfg(feature = "unstable-widget-ref")] {
    /// # use ratatui::{backend::TestBackend, Terminal};
    /// # let backend = TestBackend::new(5, 5);
    /// # let mut terminal = Terminal::new(backend).unwrap();
    /// # let mut frame = terminal.get_frame();
    /// use ratatui::{
    ///     layout::Rect,
    ///     widgets::{List, ListItem, ListState},
    /// };
    ///
    /// let mut state = ListState::default().with_selected(Some(1));
    /// let list = List::new(vec![ListItem::new("Item 1"), ListItem::new("Item 2")]);
    /// let area = Rect::new(0, 0, 5, 5);
    /// frame.render_stateful_widget_ref(list, area, &mut state);
    /// # }
    /// ```
    #[allow(clippy::needless_pass_by_value)]
    #[instability::unstable(feature = "widget-ref")]
    pub fn render_stateful_widget_ref<W>(&mut self, widget: W, area: Rect, state: &mut W::State)
    where
        W: StatefulWidgetRef,
    {
        widget.render_ref(area, self.buffer, state);
    }

    /// After drawing this frame, make the cursor visible and put it at the specified (x, y)
    /// coordinates. If this method is not called, the cursor will be hidden.
    ///
    /// Note that this will interfere with calls to [`Terminal::hide_cursor`],
    /// [`Terminal::show_cursor`], and [`Terminal::set_cursor_position`]. Pick one of the APIs and
    /// stick with it.
    ///
    /// [`Terminal::hide_cursor`]: crate::Terminal::hide_cursor
    /// [`Terminal::show_cursor`]: crate::Terminal::show_cursor
    /// [`Terminal::set_cursor_position`]: crate::Terminal::set_cursor_position
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor_position = Some(position.into());
    }

    /// After drawing this frame, make the cursor visible and put it at the specified (x, y)
    /// coordinates. If this method is not called, the cursor will be hidden.
    ///
    /// Note that this will interfere with calls to [`Terminal::hide_cursor`],
    /// [`Terminal::show_cursor`], and [`Terminal::set_cursor_position`]. Pick one of the APIs and
    /// stick with it.
    ///
    /// [`Terminal::hide_cursor`]: crate::Terminal::hide_cursor
    /// [`Terminal::show_cursor`]: crate::Terminal::show_cursor
    /// [`Terminal::set_cursor_position`]: crate::Terminal::set_cursor_position
    #[deprecated = "the method set_cursor_position indicates more clearly what about the cursor to set"]
    pub fn set_cursor(&mut self, x: u16, y: u16) {
        self.set_cursor_position(Position { x, y });
    }

    /// Gets the buffer that this `Frame` draws into as a mutable reference.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }

    /// Returns the current frame count.
    ///
    /// This method provides access to the frame count, which is a sequence number indicating
    /// how many frames have been rendered up to (but not including) this one. It can be used
    /// for purposes such as animation, performance tracking, or debugging.
    ///
    /// Each time a frame has been rendered, this count is incremented,
    /// providing a consistent way to reference the order and number of frames processed by the
    /// terminal. When count reaches its maximum value (`usize::MAX`), it wraps around to zero.
    ///
    /// This count is particularly useful when dealing with dynamic content or animations where the
    /// state of the display changes over time. By tracking the frame count, developers can
    /// synchronize updates or changes to the content with the rendering process.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ratatui::{backend::TestBackend, Terminal};
    /// # let backend = TestBackend::new(5, 5);
    /// # let mut terminal = Terminal::new(backend).unwrap();
    /// # let mut frame = terminal.get_frame();
    /// let current_count = frame.count();
    /// println!("Current frame count: {}", current_count);
    /// ```
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Sets the number of lines to scroll up and captures the current buffer state.
    ///
    /// When this is called, the terminal will use native terminal scrolling to push the
    /// captured content into the terminal's scrollback buffer before rendering the new frame.
    /// This is useful for applications like log viewers where content continuously scrolls upward.
    ///
    /// **Important**: This method captures the current buffer state at the time it's called.
    /// This means you should call it after rendering the scrollable content but before rendering
    /// any overlays (like modals or popups). The captured content is what will be pushed to
    /// scrollback, so overlays rendered after this call won't accidentally end up in scrollback.
    ///
    /// # Recommended Usage Pattern
    ///
    /// ```rust,ignore
    /// terminal.draw(|frame| {
    ///     // 1. First, render your main scrollable content
    ///     render_log_content(frame);
    ///
    ///     // 2. Call set_scroll_up to capture content for scrollback
    ///     frame.set_scroll_up(2);
    ///
    ///     // 3. Now render any overlays (modals, popups, etc.)
    ///     if show_modal {
    ///         render_modal(frame);
    ///     }
    /// })?;
    /// ```
    ///
    /// # Parameters
    ///
    /// - `lines`: Number of lines from the top of the current buffer to push into scrollback.
    ///   This value is treated as a hint - if it exceeds the viewport height, it will be clamped.
    ///
    /// # Multiple Calls
    ///
    /// If called multiple times during a single frame, subsequent calls will update the snapshot.
    /// This allows for complex scenarios where the scroll amount needs to be adjusted.
    ///
    /// This method is only available when the `native-scrolling` feature is enabled.
    #[cfg(feature = "native-scrolling")]
    pub fn set_scroll_up(&mut self, lines: u16) {
        let width = self.viewport_area.width;
        let height = self.viewport_area.height;
        let lines = lines.min(height);

        if lines == 0 {
            self.scroll_snapshot = None;
            return;
        }

        // Capture the top `lines` rows of the current buffer
        let cells_to_capture = (lines as usize) * (width as usize);
        let content = self.buffer.content[..cells_to_capture].to_vec();

        self.scroll_snapshot = Some(ScrollSnapshot {
            lines,
            content,
            width,
        });
    }
}
