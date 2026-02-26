#[derive(Debug)]
pub enum Error<E> {
    InvariantViolation,
    UserError(E),
}

pub type Result<T, E> = std::result::Result<T, Error<E>>;
