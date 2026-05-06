#[macro_export]
macro_rules! vec_of_strings {
    ( $( $s:literal ),* $(,)? ) => {
        vec![ $( $s.to_string() ),* ]
    };
}
