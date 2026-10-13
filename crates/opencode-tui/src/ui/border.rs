pub struct BorderChars {
    pub top_left: char,
    pub top_right: char,
    pub bottom_left: char,
    pub bottom_right: char,
    pub horizontal: char,
    pub vertical: char,
}

impl Clone for BorderChars {
    fn clone(&self) -> Self {
        Self {
            top_left: self.top_left,
            top_right: self.top_right,
            bottom_left: self.bottom_left,
            bottom_right: self.bottom_right,
            horizontal: self.horizontal,
            vertical: self.vertical,
        }
    }
}

impl Default for BorderChars {
    fn default() -> Self {
        Self::rounded()
    }
}

impl BorderChars {
    pub fn rounded() -> Self {
        Self {
            top_left: '╭',
            top_right: '╮',
            bottom_left: '╰',
            bottom_right: '╯',
            horizontal: '─',
            vertical: '│',
        }
    }

    pub fn squared() -> Self {
        Self {
            top_left: '┌',
            top_right: '┐',
            bottom_left: '└',
            bottom_right: '┘',
            horizontal: '─',
            vertical: '│',
        }
    }

    pub fn thick() -> Self {
        Self {
            top_left: '┏',
            top_right: '┓',
            bottom_left: '┗',
            bottom_right: '┛',
            horizontal: '━',
            vertical: '┃',
        }
    }

    pub fn empty() -> Self {
        Self {
            top_left: ' ',
            top_right: ' ',
            bottom_left: ' ',
            bottom_right: ' ',
            horizontal: ' ',
            vertical: ' ',
        }
    }

    pub fn custom(vertical: char, bottom_left: char) -> Self {
        Self {
            top_left: ' ',
            top_right: ' ',
            bottom_left,
            bottom_right: ' ',
            horizontal: ' ',
            vertical,
        }
    }
}

#[derive(Clone)]
pub enum BorderStyle {
    None,
    Single,
    Double,
    Rounded,
    Thick,
    Custom(BorderChars),
}
