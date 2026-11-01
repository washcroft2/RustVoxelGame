use crate::RenderError;

/// The adapter, device and queue, plus the capabilities the renderer cares about.
pub struct Gpu {
    /// Kept alive for as long as the device is used.
    _instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// `INDIRECT_FIRST_INSTANCE`: indirect draws may carry a first-instance value.
    pub indirect_first_instance: bool,
    pub max_storage_binding: u64,
    pub max_array_layers: u32,
}

impl Gpu {
    pub fn new(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, RenderError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|_| RenderError::NoAdapter)?;
        let features = adapter.features();
        let limits = adapter.limits();
        let wanted = wgpu::Features::INDIRECT_FIRST_INSTANCE;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features & wanted,
            required_limits: limits.clone(),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        Ok(Self {
            _instance: instance,
            indirect_first_instance: features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            max_storage_binding: limits.max_storage_buffer_binding_size,
            max_array_layers: limits.max_texture_array_layers,
            adapter,
            device,
            queue,
        })
    }

    /// One-line description for logs and the F3 overlay.
    pub fn describe(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type)
    }
}
