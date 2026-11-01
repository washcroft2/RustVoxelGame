#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no suitable graphics adapter found")]
    NoAdapter,
    #[error("could not create the graphics device: {0}")]
    Device(String),
    #[error("could not create the window surface: {0}")]
    Surface(String),
    #[error("shader or pipeline creation failed: {0}")]
    Pipeline(String),
    #[error("texture set is unusable: {0}")]
    Textures(String),
    #[error("the GPU could not give a frame to draw into")]
    NoFrame,
    #[error("reading back the image failed: {0}")]
    Readback(String),
    #[error("this renderer draws to a window, not to memory")]
    NotHeadless,
}
