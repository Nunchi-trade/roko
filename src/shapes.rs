pub trait Drawable {
    fn draw(&self) -> String;
}

pub struct Circle {
    pub radius: f64,
}

pub struct Rectangle {
    pub width: f64,
    pub height: f64,
}

impl Drawable for Circle {
    fn draw(&self) -> String {
        format!("Circle(radius: {})", self.radius)
    }
}

impl Drawable for Rectangle {
    fn draw(&self) -> String {
        format!("Rectangle(width: {}, height: {})", self.width, self.height)
    }
}
