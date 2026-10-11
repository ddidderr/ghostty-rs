//! Screen/grid/cell/padding size math (`renderer/size.zig`).

/// Terminal cell count integer (`terminal.size.CellCountInt`).
// TODO(integration): re-export from ghostty-terminal once it is a dependency.
pub type CellCountInt = u16;

/// All relevant sizes for a rendered terminal. These are all the sizes that
/// any functionality should need to know about the terminal in order to
/// convert between any coordinate systems.
///
/// See the individual field type documentation for more information on each
/// field. One important note is that any pixel values should already be scaled
/// to the current DPI of the screen. If the DPI changes, the sizes should be
/// recalculated and we expect this to be done by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub screen: ScreenSize,
    pub cell: CellSize,
    pub padding: Padding,
}

impl Size {
    /// Return the grid size for this size. The grid size is calculated by
    /// taking the screen size, removing padding, and dividing by the cell
    /// dimensions.
    pub fn grid(&self) -> GridSize {
        GridSize::init(self.screen.sub_padding(self.padding), self.cell)
    }

    /// The size of the terminal. This is the same as the screen without
    /// padding.
    pub fn terminal(&self) -> ScreenSize {
        self.screen.sub_padding(self.padding)
    }

    /// Set the padding to be balanced around the grid. The balanced
    /// padding is calculated AFTER the explicit padding is taken
    /// into account.
    pub fn balance_padding(&mut self, explicit: Padding) {
        // This ensure grid() does the right thing
        self.padding = explicit;

        // Now we can calculate the balanced padding
        self.padding = Padding::balanced(self.screen, self.grid(), self.cell);

        // The top/bottom padding is interesting. Subjectively, lots of padding
        // at the top looks bad. So instead of always being equal (like left/right),
        // we force the top padding to be at most equal to the maximum left padding,
        // which is the balanced explicit horizontal padding plus half a cell width.
        let max_padding_left = (explicit.left + explicit.right + self.cell.width) / 2;
        let vshift = self.padding.top.saturating_sub(max_padding_left);
        self.padding.top -= vshift;
        self.padding.bottom += vshift;
    }
}

/// A coordinate. This is defined as a tagged union to allow for different
/// coordinate systems to be represented.
///
/// A coordinate is only valid within the context of a stable Size value.
/// If any of the sizes in the Size struct change, the coordinate is no
/// longer valid and must be recalculated. A conversion function is provided
/// to migrate to a new Size (which may result in failure).
///
/// The coordinate systems are:
///
///   * surface: (0, 0) is the top-left of the surface (with padding). Negative
///     values are allowed and are off the surface. Likewise, values greater
///     than the surface size are off the surface. Units are pixels.
///
///   * terminal: (0, 0) is the top-left of the terminal grid. This is the
///     same as the surface but with the padding removed. Negative values and
///     values greater than the grid size are allowed and are off the terminal.
///     Units are pixels.
///
///   * grid: (0, 0) is the top-left of the grid. Units are in cells. Negative
///     values are not allowed but values greater than the grid size are
///     possible and are off the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coordinate {
    Surface(SurfaceCoord),
    Terminal(TerminalCoord),
    Grid(GridCoord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateTag {
    Surface,
    Terminal,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceCoord {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalCoord {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCoord {
    pub x: CellCountInt,
    pub y: CellCountInt,
}

impl Coordinate {
    pub fn tag(&self) -> CoordinateTag {
        match self {
            Coordinate::Surface(_) => CoordinateTag::Surface,
            Coordinate::Terminal(_) => CoordinateTag::Terminal,
            Coordinate::Grid(_) => CoordinateTag::Grid,
        }
    }

    /// Convert a coordinate to a different space within the same Size.
    pub fn convert(self, to: CoordinateTag, size: Size) -> Coordinate {
        // Unlikely fast-path but avoid work.
        if self.tag() == to {
            return self;
        }

        // To avoid the combinatorial explosion of conversion functions, we
        // convert to the surface system first and then reconvert from there.
        let surface = self.convert_to_surface(size);

        match to {
            CoordinateTag::Surface => Coordinate::Surface(surface),
            CoordinateTag::Terminal => Coordinate::Terminal(TerminalCoord {
                x: surface.x - f64::from(size.padding.left),
                y: surface.y - f64::from(size.padding.top),
            }),
            CoordinateTag::Grid => {
                // Get rid of the padding.
                let Coordinate::Terminal(term) =
                    Coordinate::Surface(surface).convert(CoordinateTag::Terminal, size)
                else {
                    unreachable!()
                };

                // We need our grid to clamp
                let grid = size.grid();

                // Calculate the grid position.
                let cell_width = f64::from(size.cell.width);
                let cell_height = f64::from(size.cell.height);
                let clamped_x = term.x.max(0.0);
                let clamped_y = term.y.max(0.0);
                let col = (clamped_x / cell_width) as CellCountInt;
                let row = (clamped_y / cell_height) as CellCountInt;
                let clamped_col = col.min(grid.columns - 1);
                let clamped_row = row.min(grid.rows - 1);
                Coordinate::Grid(GridCoord {
                    x: clamped_col,
                    y: clamped_row,
                })
            }
        }
    }

    /// Convert a coordinate to the surface coordinate system.
    fn convert_to_surface(self, size: Size) -> SurfaceCoord {
        match self {
            Coordinate::Surface(v) => v,
            Coordinate::Terminal(v) => SurfaceCoord {
                x: v.x + f64::from(size.padding.left),
                y: v.y + f64::from(size.padding.top),
            },
            Coordinate::Grid(v) => {
                let col = f64::from(v.x);
                let row = f64::from(v.y);
                let cell_width = f64::from(size.cell.width);
                let cell_height = f64::from(size.cell.height);
                let padding_left = f64::from(size.padding.left);
                let padding_top = f64::from(size.padding.top);
                SurfaceCoord {
                    x: col * cell_width + padding_left,
                    y: row * cell_height + padding_top,
                }
            }
        }
    }
}

/// The dimensions of a single "cell" in the terminal grid.
///
/// The dimensions are dependent on the current loaded set of font glyphs.
/// We calculate the width based on the widest character and the height based
/// on the height requirement for an underscore (the "lowest" -- visually --
/// character).
///
/// The units for the width and height are in world space. They have to
/// be normalized for any renderer implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

/// The dimensions of the screen that the grid is rendered to. This is the
/// terminal screen, so it is likely a subset of the window size. The dimensions
/// should be in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenSize {
    pub width: u32,
    pub height: u32,
}

impl ScreenSize {
    /// Subtract padding from the screen size.
    pub fn sub_padding(self, padding: Padding) -> ScreenSize {
        ScreenSize {
            width: self.width.saturating_sub(padding.left + padding.right),
            height: self.height.saturating_sub(padding.top + padding.bottom),
        }
    }

    /// Calculates the amount of blank space around the grid. This is possible
    /// when padding isn't balanced.
    ///
    /// The "self" screen size here should be the unpadded screen.
    pub fn blank_padding(self, padding: Padding, grid: GridSize, cell: CellSize) -> Padding {
        let grid_width = u32::from(grid.columns) * cell.width;
        let grid_height = u32::from(grid.rows) * cell.height;
        let padded_width = grid_width + (padding.left + padding.right);
        let padded_height = grid_height + (padding.top + padding.bottom);

        // Note these have to use a saturating subtraction to avoid underflow
        // because our padding can cause the padded sizes to be larger than
        // our real screen if the screen is shrunk to a minimal size such
        // as 1x1.
        let leftover_width = self.width.saturating_sub(padded_width);
        let leftover_height = self.height.saturating_sub(padded_height);

        Padding {
            top: 0,
            bottom: leftover_height,
            right: leftover_width,
            left: 0,
        }
    }

    /// Returns true if two sizes are equal.
    pub fn equals(self, other: ScreenSize) -> bool {
        self == other
    }
}

/// The dimensions of the grid itself, in rows/columns units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridSize {
    pub columns: CellCountInt,
    pub rows: CellCountInt,
}

impl GridSize {
    /// Initialize a grid size based on a screen and cell size.
    pub fn init(screen: ScreenSize, cell: CellSize) -> GridSize {
        let mut result = GridSize::default();
        result.update(screen, cell);
        result
    }

    /// Update the columns/rows for the grid based on the given screen and
    /// cell size.
    pub fn update(&mut self, screen: ScreenSize, cell: CellSize) {
        let cell_width = cell.width as f32;
        let cell_height = cell.height as f32;
        let screen_width = screen.width as f32;
        let screen_height = screen.height as f32;
        let calc_cols = (screen_width / cell_width) as CellCountInt;
        let calc_rows = (screen_height / cell_height) as CellCountInt;
        self.columns = calc_cols.max(1);
        self.rows = calc_rows.max(1);
    }

    /// Returns true if two sizes are equal.
    pub fn equals(self, other: GridSize) -> bool {
        self == other
    }
}

/// The padding to add to a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Padding {
    pub top: u32,
    pub bottom: u32,
    pub right: u32,
    pub left: u32,
}

impl Padding {
    /// Returns padding that balances the whitespace around the screen
    /// for the given grid and cell sizes.
    pub fn balanced(screen: ScreenSize, grid: GridSize, cell: CellSize) -> Padding {
        // Turn our cell sizes into floats for the math
        let cell_width = cell.width as f32;
        let cell_height = cell.height as f32;

        // The size of our full grid
        let grid_width = f32::from(grid.columns) * cell_width;
        let grid_height = f32::from(grid.rows) * cell_height;

        // The empty space to the right of a line and bottom of the last row
        let space_right = screen.width as f32 - grid_width;
        let space_bot = screen.height as f32 - grid_height;

        // The padding is split equally along both axes.
        let padding_right = (space_right / 2.0).floor();
        let padding_left = padding_right;

        let padding_bot = (space_bot / 2.0).floor();
        let padding_top = padding_bot;

        let zero = 0.0f32;
        Padding {
            top: padding_top.max(zero) as u32,
            bottom: padding_bot.max(zero) as u32,
            right: padding_right.max(zero) as u32,
            left: padding_left.max(zero) as u32,
        }
    }

    /// Equality test between two paddings.
    pub fn eql(self, other: Padding) -> bool {
        self == other
    }
}

/// Add another padding to this one (`Padding.add`).
impl std::ops::Add for Padding {
    type Output = Padding;

    fn add(self, other: Padding) -> Padding {
        Padding {
            top: self.top + other.top,
            bottom: self.bottom + other.bottom,
            right: self.right + other.right,
            left: self.left + other.left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_balanced_on_zero() {
        // On some systems, our screen can be zero-sized for a bit, and we
        // don't want to end up with negative padding.
        let grid = GridSize {
            columns: 100,
            rows: 37,
        };
        let cell = CellSize {
            width: 10,
            height: 20,
        };
        let screen = ScreenSize {
            width: 0,
            height: 0,
        };
        let padding = Padding::balanced(screen, grid, cell);
        assert_eq!(Padding::default(), padding);
    }

    #[test]
    fn grid_size_update_exact() {
        let mut grid = GridSize::default();
        grid.update(
            ScreenSize {
                width: 100,
                height: 40,
            },
            CellSize {
                width: 5,
                height: 10,
            },
        );
        assert_eq!(20, grid.columns);
        assert_eq!(4, grid.rows);
    }

    #[test]
    fn grid_size_update_rounding() {
        let mut grid = GridSize::default();
        grid.update(
            ScreenSize {
                width: 20,
                height: 40,
            },
            CellSize {
                width: 6,
                height: 15,
            },
        );
        assert_eq!(3, grid.columns);
        assert_eq!(2, grid.rows);
    }

    #[test]
    fn coordinate_conversion() {
        // A size for testing purposes. Purposely easy to calculate numbers.
        let test_size = Size {
            screen: ScreenSize {
                width: 100,
                height: 100,
            },
            cell: CellSize {
                width: 5,
                height: 10,
            },
            padding: Padding::default(),
        };

        let g = |x, y| Coordinate::Grid(GridCoord { x, y });
        let s = |x, y| Coordinate::Surface(SurfaceCoord { x, y });
        let table = [
            (g(0, 0), s(0.0, 0.0)),
            (g(1, 0), s(6.0, 0.0)),
            (g(1, 1), s(6.0, 10.0)),
            (g(0, 0), s(-10.0, -10.0)),
            (
                g(test_size.grid().columns - 1, test_size.grid().rows - 1),
                s(100_000.0, 100_000.0),
            ),
        ];

        for (expected, input) in table {
            let actual = input.convert(expected.tag(), test_size);
            assert_eq!(expected, actual);
        }
    }

    #[test]
    fn balance_padding_keeps_top_at_most_max_left() {
        let mut size = Size {
            screen: ScreenSize {
                width: 105,
                height: 200,
            },
            cell: CellSize {
                width: 10,
                height: 20,
            },
            padding: Padding::default(),
        };
        size.balance_padding(Padding::default());
        // grid is 10x10 -> 100x200; leftover width 5 => left 2, right 2.
        assert_eq!(size.padding.left, 2);
        assert_eq!(size.padding.top, 0);
    }

    #[test]
    fn padding_add() {
        let a = Padding {
            top: 1,
            bottom: 2,
            right: 3,
            left: 4,
        };
        let sum = a + a;
        assert_eq!((sum.top, sum.bottom, sum.right, sum.left), (2, 4, 6, 8));
        assert!(sum.eql(a + a));
    }

    #[test]
    fn terminal_and_blank_padding() {
        let size = Size {
            screen: ScreenSize {
                width: 110,
                height: 70,
            },
            cell: CellSize {
                width: 10,
                height: 20,
            },
            padding: Padding {
                top: 5,
                bottom: 5,
                left: 5,
                right: 5,
            },
        };
        let term = size.terminal();
        assert_eq!((term.width, term.height), (100, 60));
        let grid = size.grid();
        assert_eq!((grid.columns, grid.rows), (10, 3));
        let blank = term.blank_padding(size.padding, grid, size.cell);
        // 100 - (100 + 10) saturates to 0; 60 - (60 + 10) saturates to 0.
        assert_eq!(blank, Padding::default());
    }
}
