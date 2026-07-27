use std::io;

use crate::{
    backend::{Backend, ClearType},
    buffer::{Buffer, Cell},
    layout::{Position, Rect, Size},
    CompletedFrame, Frame, TerminalOptions, Viewport,
};

/// An interface to interact and draw [`Frame`]s on the user's terminal.
///
/// This is the main entry point for Ratatui. It is responsible for drawing and maintaining the
/// state of the buffers, cursor and viewport.
///
/// The [`Terminal`] is generic over a [`Backend`] implementation which is used to interface with
/// the underlying terminal library. The [`Backend`] trait is implemented for three popular Rust
/// terminal libraries: [Crossterm], [Termion] and [Termwiz]. See the [`backend`] module for more
/// information.
///
/// The `Terminal` struct maintains two buffers: the current and the previous.
/// When the widgets are drawn, the changes are accumulated in the current buffer.
/// At the end of each draw pass, the two buffers are compared, and only the changes
/// between these buffers are written to the terminal, avoiding any redundant operations.
/// After flushing these changes, the buffers are swapped to prepare for the next draw cycle.
///
/// The terminal also has a viewport which is the area of the terminal that is currently visible to
/// the user. It can be either fullscreen, inline or fixed. See [`Viewport`] for more information.
///
/// Applications should detect terminal resizes and call [`Terminal::draw`] to redraw the
/// application with the new size. This will automatically resize the internal buffers to match the
/// new size for inline and fullscreen viewports. Fixed viewports are not resized automatically.
///
/// # Examples
///
/// ```rust,no_run
/// use std::io::stdout;
///
/// use ratatui::{backend::CrosstermBackend, widgets::Paragraph, Terminal};
///
/// let backend = CrosstermBackend::new(stdout());
/// let mut terminal = Terminal::new(backend)?;
/// terminal.draw(|frame| {
///     let area = frame.area();
///     frame.render_widget(Paragraph::new("Hello World!"), area);
/// })?;
/// # std::io::Result::Ok(())
/// ```
///
/// [Crossterm]: https://crates.io/crates/crossterm
/// [Termion]: https://crates.io/crates/termion
/// [Termwiz]: https://crates.io/crates/termwiz
/// [`backend`]: crate::backend
/// [`Backend`]: crate::backend::Backend
/// [`Buffer`]: crate::buffer::Buffer
#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct Terminal<B>
where
    B: Backend,
{
    /// The backend used to interface with the terminal
    backend: B,
    /// Holds the results of the current and previous draw calls. The two are compared at the end
    /// of each draw pass to output the necessary updates to the terminal
    buffers: [Buffer; 2],
    /// Index of the current buffer in the previous array
    current: usize,
    /// Whether the cursor is currently hidden
    hidden_cursor: bool,
    /// Viewport
    viewport: Viewport,
    /// Area of the viewport
    viewport_area: Rect,
    /// Last known area of the terminal. Used to detect if the internal buffers have to be resized.
    last_known_area: Rect,
    /// Last known position of the cursor. Used to find the new area when the viewport is inlined
    /// and the terminal resized.
    last_known_cursor_pos: Position,
    /// Number of frames rendered up until current time.
    frame_count: usize,
}

/// Options to pass to [`Terminal::with_options`]
#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct Options {
    /// Viewport used to draw to the terminal
    pub viewport: Viewport,
}

impl<B> Drop for Terminal<B>
where
    B: Backend,
{
    fn drop(&mut self) {
        // Attempt to restore the cursor state
        if self.hidden_cursor {
            if let Err(err) = self.show_cursor() {
                eprintln!("Failed to show the cursor: {err}");
            }
        }
    }
}

impl<B> Terminal<B>
where
    B: Backend,
{
    /// Creates a new [`Terminal`] with the given [`Backend`] with a full screen viewport.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::io::stdout;
    ///
    /// use ratatui::{backend::CrosstermBackend, Terminal};
    ///
    /// let backend = CrosstermBackend::new(stdout());
    /// let terminal = Terminal::new(backend)?;
    /// # std::io::Result::Ok(())
    /// ```
    pub fn new(backend: B) -> io::Result<Self> {
        Self::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fullscreen,
            },
        )
    }

    /// Creates a new [`Terminal`] with the given [`Backend`] and [`TerminalOptions`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::io::stdout;
    ///
    /// use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal, TerminalOptions, Viewport};
    ///
    /// let backend = CrosstermBackend::new(stdout());
    /// let viewport = Viewport::Fixed(Rect::new(0, 0, 10, 10));
    /// let terminal = Terminal::with_options(backend, TerminalOptions { viewport })?;
    /// # std::io::Result::Ok(())
    /// ```
    pub fn with_options(mut backend: B, options: TerminalOptions) -> io::Result<Self> {
        let area = match options.viewport {
            Viewport::Fullscreen | Viewport::Inline(_) => {
                Rect::from((Position::ORIGIN, backend.size()?))
            }
            Viewport::Fixed(area) => area,
        };
        let (viewport_area, cursor_pos) = match options.viewport {
            Viewport::Fullscreen => (area, Position::ORIGIN),
            Viewport::Inline(height) => {
                compute_inline_size(&mut backend, height, area.as_size(), 0)?
            }
            Viewport::Fixed(area) => (area, area.as_position()),
        };
        Ok(Self {
            backend,
            buffers: [Buffer::empty(viewport_area), Buffer::empty(viewport_area)],
            current: 0,
            hidden_cursor: false,
            viewport: options.viewport,
            viewport_area,
            last_known_area: area,
            last_known_cursor_pos: cursor_pos,
            frame_count: 0,
        })
    }

    /// Get a Frame object which provides a consistent view into the terminal state for rendering.
    ///
    /// # Note
    ///
    /// This exists to support more advanced use cases. Most cases should be fine using
    /// [`Terminal::draw`].
    ///
    /// [`Terminal::get_frame`] should be used when you need direct access to the frame buffer
    /// outside of draw closure, for example:
    ///
    /// - Unit testing widgets
    /// - Buffer state inspection
    /// - Cursor manipulation
    /// - Multiple rendering passes/Buffer Manipulation
    /// - Custom frame lifecycle management
    /// - Buffer exporting
    ///
    /// # Example
    ///
    /// Getting the buffer and asserting on some cells after rendering a widget.
    ///
    /// ```rust,ignore
    /// use ratatui::{backend::TestBackend, Terminal};
    /// use ratatui::widgets::Paragraph;
    /// let backend = TestBackend::new(30, 5);
    /// let mut terminal = Terminal::new(backend).unwrap();
    /// {
    ///     let mut frame = terminal.get_frame();
    ///     frame.render_widget(Paragraph::new("Hello"), frame.area());
    /// }
    /// // When not using `draw`, present the buffer manually:
    /// terminal.flush().unwrap();
    /// terminal.swap_buffers();
    /// terminal.backend_mut().flush().unwrap();
    /// ```
    pub fn get_frame(&mut self) -> Frame<'_> {
        let count = self.frame_count;
        Frame {
            cursor_position: None,
            viewport_area: self.viewport_area,
            buffer: self.current_buffer_mut(),
            count,
            #[cfg(feature = "native-scrolling")]
            scroll_snapshot: None,
        }
    }

    /// Gets the current buffer as a mutable reference.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// Gets the backend
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Gets the backend as a mutable reference
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Obtains a difference between the previous and the current buffer and passes it to the
    /// current backend for drawing.
    pub fn flush(&mut self) -> io::Result<()> {
        let previous_buffer = &self.buffers[1 - self.current];
        let current_buffer = &self.buffers[self.current];
        let updates = previous_buffer.diff(current_buffer);
        if let Some((col, row, _)) = updates.last() {
            self.last_known_cursor_pos = Position { x: *col, y: *row };
        }
        self.backend.draw(updates.into_iter())
    }

    /// Flushes the current buffer to the terminal with native scrolling support.
    ///
    /// This method uses the captured scroll snapshot to push the correct content into the
    /// terminal's native scrollback buffer, then computes and draws only the cells that
    /// differ from what the terminal now displays.
    ///
    /// The snapshot content is written to the terminal first, ensuring that the intended
    /// content (not overlay/modal content) goes to scrollback. It is streamed from the top-left
    /// corner as normal terminal output, then extra line advances are emitted to scroll those rows
    /// into native scrollback. This relies on normal main-screen linefeed scrolling rather than
    /// scrolling-region escape sequences, because terminals differ on whether scrolling-region
    /// operations append rows to native scrollback.
    #[cfg(feature = "native-scrolling")]
    pub fn flush_with_scroll(
        &mut self,
        snapshot: crate::terminal::frame::ScrollSnapshot,
    ) -> io::Result<()> {
        let height = self.viewport_area.height;
        let width = self.viewport_area.width;
        let scroll_lines = snapshot.lines;

        if scroll_lines == 0 {
            return self.flush();
        }

        // Stream the snapshot rows as normal terminal output from the top-left corner, then emit
        // enough line advances to scroll exactly those rows into native scrollback. This leaves
        // the visible terminal area blank, so the final redraw compares against an empty screen.
        self.backend.stream_lines_to_scrollback(
            &snapshot.content,
            snapshot.width,
            scroll_lines,
            height,
            &snapshot.row_wrapped,
        )?;

        let previous_buffer = &self.buffers[1 - self.current];
        let current_buffer = &self.buffers[self.current];

        let mut updates: Vec<(u16, u16, &Cell)> = Vec::new();
        let empty_cell = Cell::EMPTY;
        let mut force_soft_wrap_continuation = false;
        for row in 0..height {
            for col in 0..width {
                let current_idx = (row as usize) * (width as usize) + (col as usize);
                let current_cell = &current_buffer.content[current_idx];
                let post_stream_cell = if (row as usize) + scroll_lines < height as usize {
                    let prev_idx =
                        ((row as usize + scroll_lines) * (width as usize)) + (col as usize);
                    &previous_buffer.content[prev_idx]
                } else {
                    &empty_cell
                };

                let force_current = force_soft_wrap_continuation && col == 0;
                if force_current {
                    force_soft_wrap_continuation = false;
                }
                if !current_cell.skip && (force_current || current_cell != post_stream_cell) {
                    updates.push((col, row, current_cell));
                    if current_cell.soft_wrap() {
                        force_soft_wrap_continuation = true;
                    }
                }
            }
        }

        if let Some((col, row, _)) = updates.last() {
            self.last_known_cursor_pos = Position { x: *col, y: *row };
        }

        self.backend.draw(updates.into_iter())
    }

    /// Updates the Terminal so that internal buffers match the requested area.
    ///
    /// Requested area will be saved to remain consistent when rendering. This leads to a full clear
    /// of the screen.
    pub fn resize(&mut self, area: Rect) -> io::Result<()> {
        let next_area = match self.viewport {
            Viewport::Inline(height) => {
                let offset_in_previous_viewport = self
                    .last_known_cursor_pos
                    .y
                    .saturating_sub(self.viewport_area.top());
                compute_inline_size(
                    &mut self.backend,
                    height,
                    area.as_size(),
                    offset_in_previous_viewport,
                )?
                .0
            }
            Viewport::Fixed(_) | Viewport::Fullscreen => area,
        };
        self.set_viewport_area(next_area);
        self.clear()?;

        self.last_known_area = area;
        Ok(())
    }

    fn set_viewport_area(&mut self, area: Rect) {
        self.buffers[self.current].resize(area);
        self.buffers[1 - self.current].resize(area);
        self.viewport_area = area;
    }

    /// Queries the backend for size and resizes if it doesn't match the previous size.
    pub fn autoresize(&mut self) -> io::Result<()> {
        // fixed viewports do not get autoresized
        if matches!(self.viewport, Viewport::Fullscreen | Viewport::Inline(_)) {
            let area = Rect::from((Position::ORIGIN, self.size()?));
            if area != self.last_known_area {
                self.resize(area)?;
            }
        }
        Ok(())
    }

    /// Draws a single frame to the terminal.
    ///
    /// Returns a [`CompletedFrame`] if successful, otherwise a [`std::io::Error`].
    ///
    /// If the render callback passed to this method can fail, use [`try_draw`] instead.
    ///
    /// Applications should call `draw` or [`try_draw`] in a loop to continuously render the
    /// terminal. These methods are the main entry points for drawing to the terminal.
    ///
    /// [`try_draw`]: Terminal::try_draw
    ///
    /// This method will:
    ///
    /// - autoresize the terminal if necessary
    /// - call the render callback, passing it a [`Frame`] reference to render to
    /// - flush the current internal state by copying the current buffer to the backend
    /// - move the cursor to the last known position if it was set during the rendering closure
    /// - return a [`CompletedFrame`] with the current buffer and the area of the terminal
    ///
    /// The [`CompletedFrame`] returned by this method can be useful for debugging or testing
    /// purposes, but it is often not used in regular applicationss.
    ///
    /// The render callback should fully render the entire frame when called, including areas that
    /// are unchanged from the previous frame. This is because each frame is compared to the
    /// previous frame to determine what has changed, and only the changes are written to the
    /// terminal. If the render callback does not fully render the frame, the terminal will not be
    /// in a consistent state.
    ///
    /// # Examples
    ///
    /// ```
    /// # let backend = ratatui::backend::TestBackend::new(10, 10);
    /// # let mut terminal = ratatui::Terminal::new(backend)?;
    /// use ratatui::{layout::Position, widgets::Paragraph};
    ///
    /// // with a closure
    /// terminal.draw(|frame| {
    ///     let area = frame.area();
    ///     frame.render_widget(Paragraph::new("Hello World!"), area);
    ///     frame.set_cursor_position(Position { x: 0, y: 0 });
    /// })?;
    ///
    /// // or with a function
    /// terminal.draw(render)?;
    ///
    /// fn render(frame: &mut ratatui::Frame) {
    ///     frame.render_widget(Paragraph::new("Hello World!"), frame.area());
    /// }
    /// # std::io::Result::Ok(())
    /// ```
    pub fn draw<F>(&mut self, render_callback: F) -> io::Result<CompletedFrame<'_>>
    where
        F: FnOnce(&mut Frame),
    {
        self.try_draw(|frame| {
            render_callback(frame);
            io::Result::Ok(())
        })
    }

    /// Tries to draw a single frame to the terminal.
    ///
    /// Returns [`Result::Ok`] containing a [`CompletedFrame`] if successful, otherwise
    /// [`Result::Err`] containing the [`std::io::Error`] that caused the failure.
    ///
    /// This is the equivalent of [`Terminal::draw`] but the render callback is a function or
    /// closure that returns a `Result` instead of nothing.
    ///
    /// Applications should call `try_draw` or [`draw`] in a loop to continuously render the
    /// terminal. These methods are the main entry points for drawing to the terminal.
    ///
    /// [`draw`]: Terminal::draw
    ///
    /// This method will:
    ///
    /// - autoresize the terminal if necessary
    /// - call the render callback, passing it a [`Frame`] reference to render to
    /// - flush the current internal state by copying the current buffer to the backend
    /// - move the cursor to the last known position if it was set during the rendering closure
    /// - return a [`CompletedFrame`] with the current buffer and the area of the terminal
    ///
    /// The render callback passed to `try_draw` can return any [`Result`] with an error type that
    /// can be converted into an [`std::io::Error`] using the [`Into`] trait. This makes it possible
    /// to use the `?` operator to propagate errors that occur during rendering. If the render
    /// callback returns an error, the error will be returned from `try_draw` as an
    /// [`std::io::Error`] and the terminal will not be updated.
    ///
    /// The [`CompletedFrame`] returned by this method can be useful for debugging or testing
    /// purposes, but it is often not used in regular applicationss.
    ///
    /// The render callback should fully render the entire frame when called, including areas that
    /// are unchanged from the previous frame. This is because each frame is compared to the
    /// previous frame to determine what has changed, and only the changes are written to the
    /// terminal. If the render function does not fully render the frame, the terminal will not be
    /// in a consistent state.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// # use ratatui::layout::Position;;
    /// # let backend = ratatui::backend::TestBackend::new(10, 10);
    /// # let mut terminal = ratatui::Terminal::new(backend)?;
    /// use std::io;
    ///
    /// use ratatui::widgets::Paragraph;
    ///
    /// // with a closure
    /// terminal.try_draw(|frame| {
    ///     let value: u8 = "not a number".parse().map_err(io::Error::other)?;
    ///     let area = frame.area();
    ///     frame.render_widget(Paragraph::new("Hello World!"), area);
    ///     frame.set_cursor_position(Position { x: 0, y: 0 });
    ///     io::Result::Ok(())
    /// })?;
    ///
    /// // or with a function
    /// terminal.try_draw(render)?;
    ///
    /// fn render(frame: &mut ratatui::Frame) -> io::Result<()> {
    ///     let value: u8 = "not a number".parse().map_err(io::Error::other)?;
    ///     frame.render_widget(Paragraph::new("Hello World!"), frame.area());
    ///     Ok(())
    /// }
    /// # io::Result::Ok(())
    /// ```
    pub fn try_draw<F, E>(&mut self, render_callback: F) -> io::Result<CompletedFrame<'_>>
    where
        F: FnOnce(&mut Frame) -> Result<(), E>,
        E: Into<io::Error>,
    {
        // Autoresize - otherwise we get glitches if shrinking or potential desync between widgets
        // and the terminal (if growing), which may OOB.
        self.autoresize()?;

        let mut frame = self.get_frame();

        render_callback(&mut frame).map_err(Into::into)?;

        // We can't change the cursor position right away because we have to flush the frame to
        // stdout first. But we also can't keep the frame around, since it holds a &mut to
        // Buffer. Thus, we're taking the important data out of the Frame and dropping it.
        let cursor_position = frame.cursor_position;

        // Extract scroll snapshot if native-scrolling feature is enabled
        #[cfg(feature = "native-scrolling")]
        let scroll_snapshot = frame.scroll_snapshot.take();

        // Draw to stdout, using native scrolling if scroll snapshot was captured
        #[cfg(feature = "native-scrolling")]
        if let Some(snapshot) = scroll_snapshot {
            self.flush_with_scroll(snapshot)?;
        } else {
            self.flush()?;
        }
        #[cfg(not(feature = "native-scrolling"))]
        self.flush()?;

        match cursor_position {
            None => self.hide_cursor()?,
            Some(position) => {
                self.show_cursor()?;
                self.set_cursor_position(position)?;
            }
        }

        self.swap_buffers();

        // Flush
        self.backend.flush()?;

        let completed_frame = CompletedFrame {
            buffer: &self.buffers[1 - self.current],
            area: self.last_known_area,
            count: self.frame_count,
        };

        // increment frame count before returning from draw
        self.frame_count = self.frame_count.wrapping_add(1);

        Ok(completed_frame)
    }

    /// Hides the cursor.
    pub fn hide_cursor(&mut self) -> io::Result<()> {
        self.backend.hide_cursor()?;
        self.hidden_cursor = true;
        Ok(())
    }

    /// Shows the cursor.
    pub fn show_cursor(&mut self) -> io::Result<()> {
        self.backend.show_cursor()?;
        self.hidden_cursor = false;
        Ok(())
    }

    /// Gets the current cursor position.
    ///
    /// This is the position of the cursor after the last draw call and is returned as a tuple of
    /// `(x, y)` coordinates.
    #[deprecated = "the method get_cursor_position indicates more clearly what about the cursor to get"]
    pub fn get_cursor(&mut self) -> io::Result<(u16, u16)> {
        let Position { x, y } = self.get_cursor_position()?;
        Ok((x, y))
    }

    /// Sets the cursor position.
    #[deprecated = "the method set_cursor_position indicates more clearly what about the cursor to set"]
    pub fn set_cursor(&mut self, x: u16, y: u16) -> io::Result<()> {
        self.set_cursor_position(Position { x, y })
    }

    /// Gets the current cursor position.
    ///
    /// This is the position of the cursor after the last draw call.
    pub fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.backend.get_cursor_position()
    }

    /// Sets the cursor position.
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        self.backend.set_cursor_position(position)?;
        self.last_known_cursor_pos = position;
        Ok(())
    }

    /// Clear the terminal and force a full redraw on the next draw call.
    pub fn clear(&mut self) -> io::Result<()> {
        match self.viewport {
            Viewport::Fullscreen => self.backend.clear_region(ClearType::All)?,
            Viewport::Inline(_) => {
                self.backend
                    .set_cursor_position(self.viewport_area.as_position())?;
                self.backend.clear_region(ClearType::AfterCursor)?;
            }
            Viewport::Fixed(_) => {
                let area = self.viewport_area;
                for y in area.top()..area.bottom() {
                    self.backend.set_cursor_position(Position { x: 0, y })?;
                    self.backend.clear_region(ClearType::AfterCursor)?;
                }
            }
        }
        // Invalidate the back buffer to make sure the next update redraws everything, including
        // blank cells. A plain reset would make blank cells compare equal and skip drawing.
        self.buffers[1 - self.current].invalidate();
        Ok(())
    }

    /// Clears the inactive buffer and swaps it with the current buffer
    pub fn swap_buffers(&mut self) {
        self.buffers[1 - self.current].reset();
        self.current = 1 - self.current;
    }

    /// Queries the real size of the backend.
    pub fn size(&self) -> io::Result<Size> {
        self.backend.size()
    }

    /// Insert some content before the current inline viewport. This has no effect when the
    /// viewport is not inline.
    ///
    /// The `draw_fn` closure will be called to draw into a writable `Buffer` that is `height`
    /// lines tall. The content of that `Buffer` will then be inserted before the viewport.
    ///
    /// If the viewport isn't yet at the bottom of the screen, inserted lines will push it towards
    /// the bottom. Once the viewport is at the bottom of the screen, inserted lines will scroll
    /// the area of the screen above the viewport upwards.
    ///
    /// Before:
    /// ```ignore
    /// +---------------------+
    /// | pre-existing line 1 |
    /// | pre-existing line 2 |
    /// +---------------------+
    /// |       viewport      |
    /// +---------------------+
    /// |                     |
    /// |                     |
    /// +---------------------+
    /// ```
    ///
    /// After inserting 2 lines:
    /// ```ignore
    /// +---------------------+
    /// | pre-existing line 1 |
    /// | pre-existing line 2 |
    /// |   inserted line 1   |
    /// |   inserted line 2   |
    /// +---------------------+
    /// |       viewport      |
    /// +---------------------+
    /// +---------------------+
    /// ```
    ///
    /// After inserting 2 more lines:
    /// ```ignore
    /// +---------------------+
    /// | pre-existing line 2 |
    /// |   inserted line 1   |
    /// |   inserted line 2   |
    /// |   inserted line 3   |
    /// |   inserted line 4   |
    /// +---------------------+
    /// |       viewport      |
    /// +---------------------+
    /// ```
    ///
    /// If more lines are inserted than there is space on the screen, then the top lines will go
    /// directly into the terminal's scrollback buffer. At the limit, if the viewport takes up the
    /// whole screen, all lines will be inserted directly into the scrollback buffer.
    ///
    /// # Examples
    ///
    /// ## Insert a single line before the current viewport
    ///
    /// ```rust
    /// use ratatui::{
    ///     backend::TestBackend,
    ///     style::{Color, Style},
    ///     text::{Line, Span},
    ///     widgets::{Paragraph, Widget},
    ///     Terminal,
    /// };
    /// # let backend = TestBackend::new(10, 10);
    /// # let mut terminal = Terminal::new(backend).unwrap();
    /// terminal.insert_before(1, |buf| {
    ///     Paragraph::new(Line::from(vec![
    ///         Span::raw("This line will be added "),
    ///         Span::styled("before", Style::default().fg(Color::Blue)),
    ///         Span::raw(" the current viewport"),
    ///     ]))
    ///     .render(buf.area, buf);
    /// });
    /// ```
    pub fn insert_before<F>(&mut self, height: u16, draw_fn: F) -> io::Result<()>
    where
        F: FnOnce(&mut Buffer),
    {
        match self.viewport {
            #[cfg(feature = "scrolling-regions")]
            Viewport::Inline(_) => self.insert_before_scrolling_regions(height, draw_fn),
            #[cfg(not(feature = "scrolling-regions"))]
            Viewport::Inline(_) => self.insert_before_no_scrolling_regions(height, draw_fn),
            _ => Ok(()),
        }
    }

    /// Implement `Self::insert_before` using standard backend capabilities.
    #[cfg(not(feature = "scrolling-regions"))]
    fn insert_before_no_scrolling_regions(
        &mut self,
        height: u16,
        draw_fn: impl FnOnce(&mut Buffer),
    ) -> io::Result<()> {
        // The approach of this function is to first render all of the lines to insert into a
        // temporary buffer, and then to loop drawing chunks from the buffer to the screen. drawing
        // this buffer onto the screen.
        let area = Rect {
            x: 0,
            y: 0,
            width: self.viewport_area.width,
            height,
        };
        let mut buffer = Buffer::empty(area);
        draw_fn(&mut buffer);
        let mut buffer = buffer.content.as_slice();

        // Use i32 variables so we don't have worry about overflowed u16s when adding, or about
        // negative results when subtracting.
        let mut drawn_height: i32 = self.viewport_area.top().into();
        let mut buffer_height: i32 = height.into();
        let viewport_height: i32 = self.viewport_area.height.into();
        let screen_height: i32 = self.last_known_area.height.into();

        // The algorithm here is to loop, drawing large chunks of text (up to a screen-full at a
        // time), until the remainder of the buffer plus the viewport fits on the screen. We choose
        // this loop condition because it guarantees that we can write the remainder of the buffer
        // with just one call to Self::draw_lines().
        while buffer_height + viewport_height > screen_height {
            // We will draw as much of the buffer as possible on this iteration in order to make
            // forward progress. So we have:
            //
            //     to_draw = min(buffer_height, screen_height)
            //
            // We may need to scroll the screen up to make room to draw. We choose the minimal
            // possible scroll amount so we don't end up with the viewport sitting in the middle of
            // the screen when this function is done. The amount to scroll by is:
            //
            //     scroll_up = max(0, drawn_height + to_draw - screen_height)
            //
            // We want `scroll_up` to be enough so that, after drawing, we have used the whole
            // screen (drawn_height - scroll_up + to_draw = screen_height). However, there might
            // already be enough room on the screen to draw without scrolling (drawn_height +
            // to_draw <= screen_height). In this case, we just don't scroll at all.
            let to_draw = buffer_height.min(screen_height);
            let scroll_up = 0.max(drawn_height + to_draw - screen_height);
            self.scroll_up(scroll_up as u16)?;
            buffer = self.draw_lines((drawn_height - scroll_up) as u16, to_draw as u16, buffer)?;
            drawn_height += to_draw - scroll_up;
            buffer_height -= to_draw;
        }

        // There is now enough room on the screen for the remaining buffer plus the viewport,
        // though we may still need to scroll up some of the existing text first. It's possible
        // that by this point we've drained the buffer, but we may still need to scroll up to make
        // room for the viewport.
        //
        // We want to scroll up the exact amount that will leave us completely filling the screen.
        // However, it's possible that the viewport didn't start on the bottom of the screen and
        // the added lines weren't enough to push it all the way to the bottom. We deal with this
        // case by just ensuring that our scroll amount is non-negative.
        //
        // We want:
        //   screen_height = drawn_height - scroll_up + buffer_height + viewport_height
        // Or, equivalently:
        //   scroll_up = drawn_height + buffer_height + viewport_height - screen_height
        let scroll_up = 0.max(drawn_height + buffer_height + viewport_height - screen_height);
        self.scroll_up(scroll_up as u16)?;
        self.draw_lines(
            (drawn_height - scroll_up) as u16,
            buffer_height as u16,
            buffer,
        )?;
        drawn_height += buffer_height - scroll_up;

        self.set_viewport_area(Rect {
            y: drawn_height as u16,
            ..self.viewport_area
        });

        // Clear the viewport off the screen. We didn't clear earlier for two reasons. First, it
        // wasn't necessary because the buffer we drew out of isn't sparse, so it overwrote
        // whatever was on the screen. Second, there is a weird bug with tmux where a full screen
        // clear plus immediate scrolling causes some garbage to go into the scrollback.
        self.clear()?;

        Ok(())
    }

    /// Implement `Self::insert_before` using scrolling regions.
    ///
    /// If a terminal supports scrolling regions, it means that we can define a subset of rows of
    /// the screen, and then tell the terminal to scroll up or down just within that region. The
    /// rows outside of the region are not affected.
    ///
    /// This function utilizes this feature to avoid having to redraw the viewport. This is done
    /// either by splitting the screen at the top of the viewport, and then creating a gap by
    /// either scrolling the viewport down, or scrolling the area above it up. The lines to insert
    /// are then drawn into the gap created.
    #[cfg(feature = "scrolling-regions")]
    fn insert_before_scrolling_regions(
        &mut self,
        mut height: u16,
        draw_fn: impl FnOnce(&mut Buffer),
    ) -> io::Result<()> {
        // The approach of this function is to first render all of the lines to insert into a
        // temporary buffer, and then to loop drawing chunks from the buffer to the screen. drawing
        // this buffer onto the screen.
        let area = Rect {
            x: 0,
            y: 0,
            width: self.viewport_area.width,
            height,
        };
        let mut buffer = Buffer::empty(area);
        draw_fn(&mut buffer);
        let mut buffer = buffer.content.as_slice();

        // Handle the special case where the viewport takes up the whole screen.
        if self.viewport_area.height == self.last_known_area.height {
            // "Borrow" the top line of the viewport. Draw over it, then immediately scroll it into
            // scrollback. Do this repeatedly until the whole buffer has been put into scrollback.
            let mut first = true;
            while !buffer.is_empty() {
                buffer = if first {
                    self.draw_lines(0, 1, buffer)?
                } else {
                    self.draw_lines_over_cleared(0, 1, buffer)?
                };
                first = false;
                self.backend.scroll_region_up(0..1, 1)?;
            }

            // Redraw the top line of the viewport.
            let width = self.viewport_area.width as usize;
            let top_line = self.buffers[1 - self.current].content[0..width].to_vec();
            self.draw_lines_over_cleared(0, 1, &top_line)?;
            return Ok(());
        }

        // Handle the case where the viewport isn't yet at the bottom of the screen.
        {
            let viewport_top = self.viewport_area.top();
            let viewport_bottom = self.viewport_area.bottom();
            let screen_bottom = self.last_known_area.bottom();
            if viewport_bottom < screen_bottom {
                let to_draw = height.min(screen_bottom - viewport_bottom);
                self.backend
                    .scroll_region_down(viewport_top..viewport_bottom + to_draw, to_draw)?;
                buffer = self.draw_lines_over_cleared(viewport_top, to_draw, buffer)?;
                self.set_viewport_area(Rect {
                    y: viewport_top + to_draw,
                    ..self.viewport_area
                });
                height -= to_draw;
            }
        }

        let viewport_top = self.viewport_area.top();
        while height > 0 {
            let to_draw = height.min(viewport_top);
            self.backend.scroll_region_up(0..viewport_top, to_draw)?;
            buffer = self.draw_lines_over_cleared(viewport_top - to_draw, to_draw, buffer)?;
            height -= to_draw;
        }

        Ok(())
    }

    /// Draw lines at the given vertical offset. The slice of cells must contain enough cells
    /// for the requested lines. A slice of the unused cells are returned.
    fn draw_lines<'a>(
        &mut self,
        y_offset: u16,
        lines_to_draw: u16,
        cells: &'a [Cell],
    ) -> io::Result<&'a [Cell]> {
        let width: usize = self.last_known_area.width.into();
        let (to_draw, remainder) = cells.split_at(width * lines_to_draw as usize);
        if lines_to_draw > 0 {
            let iter = to_draw
                .iter()
                .enumerate()
                .map(|(i, c)| ((i % width) as u16, y_offset + (i / width) as u16, c));
            self.backend.draw(iter)?;
            self.backend.flush()?;
        }
        Ok(remainder)
    }

    /// Draw lines at the given vertical offset, assuming that the lines they are replacing on the
    /// screen are cleared. The slice of cells must contain enough cells for the requested lines. A
    /// slice of the unused cells are returned.
    #[cfg(feature = "scrolling-regions")]
    fn draw_lines_over_cleared<'a>(
        &mut self,
        y_offset: u16,
        lines_to_draw: u16,
        cells: &'a [Cell],
    ) -> io::Result<&'a [Cell]> {
        let width: usize = self.last_known_area.width.into();
        let (to_draw, remainder) = cells.split_at(width * lines_to_draw as usize);
        if lines_to_draw > 0 {
            let area = Rect::new(0, y_offset, width as u16, y_offset + lines_to_draw);
            let old = Buffer::empty(area);
            let new = Buffer {
                area,
                content: to_draw.to_vec(),
            };
            self.backend.draw(old.diff(&new).into_iter())?;
            self.backend.flush()?;
        }
        Ok(remainder)
    }

    /// Scroll the whole screen up by the given number of lines.
    #[cfg(not(feature = "scrolling-regions"))]
    fn scroll_up(&mut self, lines_to_scroll: u16) -> io::Result<()> {
        if lines_to_scroll > 0 {
            self.set_cursor_position(Position::new(
                0,
                self.last_known_area.height.saturating_sub(1),
            ))?;
            self.backend.append_lines(lines_to_scroll)?;
        }
        Ok(())
    }
}

fn compute_inline_size<B: Backend>(
    backend: &mut B,
    height: u16,
    size: Size,
    offset_in_previous_viewport: u16,
) -> io::Result<(Rect, Position)> {
    let pos = backend.get_cursor_position()?;
    let mut row = pos.y;

    let max_height = size.height.min(height);

    let lines_after_cursor = height
        .saturating_sub(offset_in_previous_viewport)
        .saturating_sub(1);

    backend.append_lines(lines_after_cursor)?;

    let available_lines = size.height.saturating_sub(row).saturating_sub(1);
    let missing_lines = lines_after_cursor.saturating_sub(available_lines);
    if missing_lines > 0 {
        row = row.saturating_sub(missing_lines);
    }
    row = row.saturating_sub(offset_in_previous_viewport);

    Ok((
        Rect {
            x: 0,
            y: row,
            width: size.width,
            height: max_height,
        },
        pos,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::TestBackend;
    use crate::buffer::Buffer;
    use crate::widgets::Widget;

    /// A simple widget that fills the area with a single character
    struct FillWidget(char);

    impl Widget for FillWidget {
        fn render(self, area: Rect, buf: &mut Buffer) {
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    buf[(x, y)].set_symbol(&self.0.to_string());
                }
            }
        }
    }

    #[test]
    fn basic_draw() {
        let backend = TestBackend::new(5, 3);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(FillWidget('a'), frame.area());
            })
            .unwrap();

        terminal
            .backend()
            .assert_buffer_lines(["aaaaa", "aaaaa", "aaaaa"]);
    }

    #[cfg(feature = "native-scrolling")]
    mod scroll_up_tests {
        use super::*;
        use std::io;

        use crate::backend::{Backend, ClearType, WindowSize};
        use crate::layout::{Position, Size};

        struct ScrollSpyBackend {
            inner: TestBackend,
            draw_calls: usize,
            draw_update_counts: Vec<usize>,
            draw_updates: Vec<Vec<(u16, u16, Cell)>>,
            append_lines_calls: Vec<u16>,
            scroll_region_up_calls: Vec<(std::ops::Range<u16>, u16)>,
            stream_lines_to_scrollback_calls: Vec<(u16, usize, u16)>,
        }

        impl ScrollSpyBackend {
            fn new(width: u16, height: u16) -> Self {
                Self {
                    inner: TestBackend::new(width, height),
                    draw_calls: 0,
                    draw_update_counts: Vec::new(),
                    draw_updates: Vec::new(),
                    append_lines_calls: Vec::new(),
                    scroll_region_up_calls: Vec::new(),
                    stream_lines_to_scrollback_calls: Vec::new(),
                }
            }
        }

        impl Backend for ScrollSpyBackend {
            fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
            where
                I: Iterator<Item = (u16, u16, &'a Cell)>,
            {
                self.draw_calls += 1;
                let updates = content
                    .map(|(x, y, cell)| (x, y, cell.clone()))
                    .collect::<Vec<_>>();
                self.draw_update_counts.push(updates.len());
                self.draw_updates.push(updates.clone());
                self.inner
                    .draw(updates.iter().map(|(x, y, cell)| (*x, *y, cell)))
            }

            fn append_lines(&mut self, n: u16) -> io::Result<()> {
                self.append_lines_calls.push(n);
                self.inner.append_lines(n)
            }

            fn stream_lines_to_scrollback(
                &mut self,
                content: &[Cell],
                width: u16,
                line_count: usize,
                screen_height: u16,
                row_wrapped: &[bool],
            ) -> io::Result<()> {
                self.stream_lines_to_scrollback_calls
                    .push((width, line_count, screen_height));
                self.inner.stream_lines_to_scrollback(
                    content,
                    width,
                    line_count,
                    screen_height,
                    row_wrapped,
                )
            }

            fn hide_cursor(&mut self) -> io::Result<()> {
                self.inner.hide_cursor()
            }

            fn show_cursor(&mut self) -> io::Result<()> {
                self.inner.show_cursor()
            }

            fn get_cursor_position(&mut self) -> io::Result<Position> {
                self.inner.get_cursor_position()
            }

            fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
                self.inner.set_cursor_position(position)
            }

            fn clear(&mut self) -> io::Result<()> {
                self.inner.clear()
            }

            fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
                self.inner.clear_region(clear_type)
            }

            fn size(&self) -> io::Result<Size> {
                self.inner.size()
            }

            fn window_size(&mut self) -> io::Result<WindowSize> {
                self.inner.window_size()
            }

            fn flush(&mut self) -> io::Result<()> {
                self.inner.flush()
            }

            #[cfg(feature = "scrolling-regions")]
            fn scroll_region_up(
                &mut self,
                region: std::ops::Range<u16>,
                line_count: u16,
            ) -> io::Result<()> {
                self.scroll_region_up_calls
                    .push((region.clone(), line_count));
                self.inner.scroll_region_up(region, line_count)
            }

            #[cfg(feature = "scrolling-regions")]
            fn scroll_region_down(
                &mut self,
                region: std::ops::Range<u16>,
                line_count: u16,
            ) -> io::Result<()> {
                self.inner.scroll_region_down(region, line_count)
            }
        }

        /// A widget that fills each row with a different character starting from given offset
        struct RowFillWidgetFrom(u8);

        impl Widget for RowFillWidgetFrom {
            fn render(self, area: Rect, buf: &mut Buffer) {
                for y in area.top()..area.bottom() {
                    let ch = (self.0 + (y - area.top()) as u8) as char;
                    for x in area.left()..area.right() {
                        buf[(x, y)].set_symbol(&ch.to_string());
                    }
                }
            }
        }

        #[test]
        fn set_scroll_up_basic() {
            let backend = TestBackend::new(5, 4);
            let mut terminal = Terminal::new(backend).unwrap();

            // First draw: fill with a, b, c, d
            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .assert_buffer_lines(["aaaaa", "bbbbb", "ccccc", "ddddd"]);
            terminal.backend().assert_scrollback_empty();

            // Second draw: scroll up by 2 lines
            // First render the content to be scrolled (a, b), then capture with set_scroll_up,
            // then render the new content (c, d, e, f)
            terminal
                .draw(|frame| {
                    // Step 1: Render content that should go to scrollback
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                    // Step 2: Capture the top 2 rows for scrollback
                    frame.set_scroll_up(2);
                    // Step 3: Render the new frame content (c, d, e, f)
                    frame.render_widget(RowFillWidgetFrom(b'c'), frame.area());
                })
                .unwrap();

            // After scrolling up by 2:
            // - Captured rows (aaaaa, bbbbb) should be in scrollback
            // - Screen should show c, d, e, f
            terminal
                .backend()
                .assert_buffer_lines(["ccccc", "ddddd", "eeeee", "fffff"]);
            terminal
                .backend()
                .assert_scrollback_lines(["aaaaa", "bbbbb"]);
        }

        #[test]
        fn set_scroll_up_streams_snapshot_without_absolute_draws() {
            let backend = ScrollSpyBackend::new(5, 4);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                    frame.set_scroll_up(2);
                    frame.render_widget(RowFillWidgetFrom(b'c'), frame.area());
                })
                .unwrap();

            let backend = terminal.backend();
            assert_eq!(backend.stream_lines_to_scrollback_calls, vec![(5, 2, 4)]);
            assert!(backend.append_lines_calls.is_empty());
            assert!(backend.scroll_region_up_calls.is_empty());
            // One draw for the initial frame and one draw for the final redraw after streaming.
            // The scrollback snapshot itself must not be written with absolute-positioned draws.
            assert_eq!(backend.draw_calls, 2);
            backend
                .inner
                .assert_buffer_lines(["ccccc", "ddddd", "eeeee", "fffff"]);
            backend.inner.assert_scrollback_lines(["aaaaa", "bbbbb"]);
        }

        #[test]
        fn set_scroll_snapshot_streams_more_than_viewport_height() {
            let backend = ScrollSpyBackend::new(5, 4);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| {
                    let mut content = Vec::new();
                    for row in 0..6 {
                        let symbol = ((b'a' + row) as char).to_string();
                        for _ in 0..5 {
                            let mut cell = Cell::default();
                            cell.set_symbol(&symbol);
                            content.push(cell);
                        }
                    }

                    frame.set_scroll_snapshot(content, 5, 6, vec![false; 6]);
                    frame.render_widget(RowFillWidgetFrom(b'g'), frame.area());
                })
                .unwrap();

            let backend = terminal.backend();
            assert_eq!(backend.stream_lines_to_scrollback_calls, vec![(5, 6, 4)]);
            backend
                .inner
                .assert_scrollback_lines(["aaaaa", "bbbbb", "ccccc", "ddddd", "eeeee", "fffff"]);
            backend
                .inner
                .assert_buffer_lines(["ggggg", "hhhhh", "iiiii", "jjjjj"]);
        }

        #[test]
        fn set_scroll_up_diffs_against_post_stream_screen_state() {
            let backend = ScrollSpyBackend::new(5, 4);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                    frame.set_scroll_up(2);
                    // Rows c/d match what the terminal will physically show after streaming:
                    // old rows c/d shifted to the top. Rows e/f must still be drawn.
                    frame.render_widget(RowFillWidgetFrom(b'c'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .inner
                .assert_buffer_lines(["ccccc", "ddddd", "eeeee", "fffff"]);
            terminal
                .backend()
                .inner
                .assert_scrollback_lines(["aaaaa", "bbbbb"]);
            let final_updates = terminal.backend().draw_updates.last().unwrap();
            assert!(
                final_updates.iter().all(|(_, y, _)| *y >= 2),
                "rows already present after streaming should not be redrawn"
            );
            assert!(
                final_updates
                    .iter()
                    .any(|(_, y, cell)| *y == 2 && cell.symbol() == "e"),
                "row e must be drawn after streaming"
            );
            assert!(
                final_updates
                    .iter()
                    .any(|(_, y, cell)| *y == 3 && cell.symbol() == "f"),
                "row f must be drawn after streaming"
            );
        }

        #[test]
        fn set_scroll_up_full_screen() {
            let backend = TestBackend::new(4, 3);
            let mut terminal = Terminal::new(backend).unwrap();

            // First draw
            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .assert_buffer_lines(["aaaa", "bbbb", "cccc"]);

            // Scroll entire screen
            terminal
                .draw(|frame| {
                    // Render content to be scrolled
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                    // Capture all 3 rows
                    frame.set_scroll_up(3);
                    // Render new content
                    frame.render_widget(FillWidget('x'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .assert_buffer_lines(["xxxx", "xxxx", "xxxx"]);
            terminal
                .backend()
                .assert_scrollback_lines(["aaaa", "bbbb", "cccc"]);
        }

        #[test]
        fn set_scroll_up_with_modal_overlay() {
            // Test that modals rendered AFTER set_scroll_up don't go to scrollback
            let backend = TestBackend::new(5, 3);
            let mut terminal = Terminal::new(backend).unwrap();

            // First draw: log content
            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .assert_buffer_lines(["aaaaa", "bbbbb", "ccccc"]);

            // Second draw: scroll by 1, then overlay a "modal" on row 1
            terminal
                .draw(|frame| {
                    // Render the content to be scrolled (row 'a')
                    for x in 0..5 {
                        frame.buffer_mut()[(x, 0)].set_symbol("a");
                    }
                    // Capture row 0 for scrollback
                    frame.set_scroll_up(1);
                    // Now render new content
                    frame.render_widget(RowFillWidgetFrom(b'b'), frame.area());
                    // Render a "modal" that overlays row 1
                    for x in 0..5 {
                        frame.buffer_mut()[(x, 1)].set_symbol("M");
                    }
                })
                .unwrap();

            // Screen shows: b, M, d (modal on row 1)
            terminal
                .backend()
                .assert_buffer_lines(["bbbbb", "MMMMM", "ddddd"]);
            // Scrollback has 'a' (the captured content, not the modal)
            terminal.backend().assert_scrollback_lines(["aaaaa"]);
        }

        #[test]
        fn set_scroll_up_zero_is_noop() {
            let backend = TestBackend::new(4, 2);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            // Scroll by 0 should have no effect
            terminal
                .draw(|frame| {
                    frame.set_scroll_up(0);
                    frame.render_widget(FillWidget('x'), frame.area());
                })
                .unwrap();

            terminal.backend().assert_buffer_lines(["xxxx", "xxxx"]);
            terminal.backend().assert_scrollback_empty();
        }

        #[test]
        fn set_scroll_up_clamped_to_height() {
            let backend = TestBackend::new(4, 3);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            // Scroll by more than height should be clamped
            terminal
                .draw(|frame| {
                    // Render content to be scrolled
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                    frame.set_scroll_up(100); // Way more than height of 3
                    frame.render_widget(FillWidget('x'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .assert_buffer_lines(["xxxx", "xxxx", "xxxx"]);
            // All 3 lines should be in scrollback
            terminal
                .backend()
                .assert_scrollback_lines(["aaaa", "bbbb", "cccc"]);
        }

        #[test]
        fn set_scroll_up_multiple_calls_updates_snapshot() {
            let backend = TestBackend::new(4, 3);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| {
                    frame.render_widget(RowFillWidgetFrom(b'a'), frame.area());
                })
                .unwrap();

            terminal
                .draw(|frame| {
                    // First snapshot with 'x' content
                    for x in 0..4 {
                        frame.buffer_mut()[(x, 0)].set_symbol("x");
                    }
                    frame.set_scroll_up(1);

                    // Second snapshot overwrites with 'y' content
                    for x in 0..4 {
                        frame.buffer_mut()[(x, 0)].set_symbol("y");
                    }
                    frame.set_scroll_up(1);

                    // Final content
                    frame.render_widget(RowFillWidgetFrom(b'b'), frame.area());
                })
                .unwrap();

            terminal
                .backend()
                .assert_buffer_lines(["bbbb", "cccc", "dddd"]);
            // Scrollback should have 'y' (the last captured content)
            terminal.backend().assert_scrollback_lines(["yyyy"]);
        }
    }
}
