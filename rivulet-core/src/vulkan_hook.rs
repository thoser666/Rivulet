// G3 – Vulkan hook for zero-overhead game capture.
//
// Uses `ash` for raw Vulkan API access.
// Pipeline: Entry → Instance → Device → Queue → Swapchain → Capture.
//
// Capture flow (per frame):
//   1. Wait for previous capture (fence)
//   2. Transition swapchain image: UNDEFINED → TRANSFER_SRC_OPTIMAL
//   3. Copy image → staging buffer (HOST_VISIBLE)
//   4. Transition swapchain image: TRANSFER_SRC_OPTIMAL → PRESENT_SRC_KHR
//   5. Submit + fence
//   6. Map staging buffer → read RGBA pixels → CapturedFrame

use std::ffi::CString;

use ash::{vk, Entry, Device};

pub struct VulkanHook {
    pub entry: Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: Device,
    pub queue: vk::Queue,
    pub queue_family_index: u32,
    pub surface_loader: ash::khr::surface::Instance,
    pub swapchain_loader: ash::khr::swapchain::Device,
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: Vec<vk::Image>,
    pub swapchain_format: vk::Format,
    pub swapchain_extent: vk::Extent2D,
    pub captured_frame: Option<CapturedFrame>,
    pub backend: VulkanHookBackend,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    staging_buffer: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    staging_size: vk::DeviceSize,
    fence: vk::Fence,
}

#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanHookBackend {
    Layer,
    Detour,
}

fn format_to_raw(f: vk::Format) -> i32 {
    f.as_raw()
}

impl std::fmt::Debug for VulkanHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanHook")
            .field("physical_device", &self.physical_device)
            .field("queue_family_index", &self.queue_family_index)
            .field("swapchain_format", &format_to_raw(self.swapchain_format))
            .field("swapchain_extent", &format!("{}x{}", self.swapchain_extent.width, self.swapchain_extent.height))
            .field("swapchain_images", &self.swapchain_images.len())
            .field("backend", &self.backend)
            .finish()
    }
}

impl VulkanHook {
    pub fn new() -> anyhow::Result<Self> {
        let entry = unsafe { Entry::load()? };

        let app_name = CString::new("Rivulet")?;
        let engine_name = CString::new("Rivulet Engine")?;
        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name.as_c_str())
            .application_version(vk::make_api_version(0, 1, 0, 0))
            .engine_name(engine_name.as_c_str())
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::make_api_version(0, 1, 3, 0));

        let mut ext_names: Vec<*const std::ffi::c_char> = vec![vk::KHR_SURFACE_NAME.as_ptr()];
        #[cfg(target_os = "windows")]
        ext_names.push(vk::KHR_WIN32_SURFACE_NAME.as_ptr());
        #[cfg(target_os = "linux")]
        ext_names.push(vk::KHR_XLIB_SURFACE_NAME.as_ptr());

        let instance_ci = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&ext_names);

        let instance = unsafe { entry.create_instance(&instance_ci, None)? };

        let phys_devices = unsafe { instance.enumerate_physical_devices()? };
        if phys_devices.is_empty() {
            return Err(anyhow::anyhow!("No Vulkan-capable GPU found"));
        }
        let physical_device = *phys_devices
            .iter()
            .min_by_key(|pd| {
                let props = unsafe { instance.get_physical_device_properties(**pd) };
                match props.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 0u32,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                    _ => 2,
                }
            })
            .unwrap();

        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let queue_family_index = queue_families
            .iter()
            .position(|qf| qf.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .map(|i| i as u32)
            .ok_or_else(|| anyhow::anyhow!("No graphics queue family found"))?;

        let queue_priority = 1.0f32;
        let queue_priorities = [queue_priority];
        let queue_ci = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priorities);

        let dev_ext_names: Vec<*const std::ffi::c_char> = vec![vk::KHR_SWAPCHAIN_NAME.as_ptr()];

        let device_ci = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_ci))
            .enabled_extension_names(&dev_ext_names);

        let device = unsafe { instance.create_device(physical_device, &device_ci, None)? };
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);

        let command_pool = unsafe {
            let pool_ci = vk::CommandPoolCreateInfo::default()
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                .queue_family_index(queue_family_index);
            device.create_command_pool(&pool_ci, None)?
        };

        let command_buffer = unsafe {
            let alloc_ci = vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            device.allocate_command_buffers(&alloc_ci)?[0]
        };

        let fence = unsafe {
            let fence_ci = vk::FenceCreateInfo::default()
                .flags(vk::FenceCreateFlags::SIGNALED);
            device.create_fence(&fence_ci, None)?
        };

        Ok(Self {
            entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
            surface_loader,
            swapchain_loader,
            swapchain: vk::SwapchainKHR::null(),
            swapchain_images: Vec::new(),
            swapchain_format: vk::Format::UNDEFINED,
            swapchain_extent: vk::Extent2D { width: 0, height: 0 },
            captured_frame: None,
            backend: VulkanHookBackend::Layer,
            memory_properties,
            command_pool,
            command_buffer,
            staging_buffer: vk::Buffer::null(),
            staging_memory: vk::DeviceMemory::null(),
            staging_size: 0,
            fence,
        })
    }

    pub fn has_swapchain(&self) -> bool {
        self.swapchain != vk::SwapchainKHR::null()
    }

    pub fn device_name(&self) -> String {
        unsafe {
            let props = self.instance.get_physical_device_properties(self.physical_device);
            let name_bytes: Vec<u8> = props.device_name.iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8)
                .collect();
            String::from_utf8_lossy(&name_bytes).to_string()
        }
    }

    pub fn api_version(&self) -> (u32, u32, u32) {
        unsafe {
            let props = self.instance.get_physical_device_properties(self.physical_device);
            (
                vk::api_version_major(props.api_version),
                vk::api_version_minor(props.api_version),
                vk::api_version_patch(props.api_version),
            )
        }
    }

    pub fn queue_family_supports_transfer(&self) -> bool {
        unsafe {
            let queue_families = self.instance
                .get_physical_device_queue_family_properties(self.physical_device);
            queue_families.iter().any(|qf| qf.queue_flags.contains(vk::QueueFlags::TRANSFER))
        }
    }

    pub fn image_format_is_rgba(&self) -> bool {
        matches!(
            self.swapchain_format,
            vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB
        )
    }

    pub fn image_format_is_bgra(&self) -> bool {
        matches!(
            self.swapchain_format,
            vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
        )
    }

    /// Creates a swapchain for a given surface and extent.
    /// After creating the swapchain, initializes the staging buffer.
    ///
    /// # Safety
    /// `surface` must be a valid VkSurfaceKHR created from this instance.
    pub unsafe fn create_swapchain(
        &mut self,
        surface: vk::SurfaceKHR,
        extent: vk::Extent2D,
    ) -> anyhow::Result<()> {
        let surface_caps = self.surface_loader
            .get_physical_device_surface_capabilities(self.physical_device, surface)
            .map_err(|e| anyhow::anyhow!("Surface capabilities query failed: {:?}", e))?;

        let formats = self.surface_loader
            .get_physical_device_surface_formats(self.physical_device, surface)
            .map_err(|e| anyhow::anyhow!("Surface formats query failed: {:?}", e))?;

        let format = formats.iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .or(formats.first())
            .ok_or_else(|| anyhow::anyhow!("No surface formats available"))?;

        let present_modes = self.surface_loader
            .get_physical_device_surface_present_modes(self.physical_device, surface)
            .map_err(|e| anyhow::anyhow!("Surface present modes query failed: {:?}", e))?;

        let present_mode = present_modes.iter()
            .find(|&&m| m == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(&vk::PresentModeKHR::FIFO);

        let image_count = (surface_caps.min_image_count + 1)
            .min(surface_caps.max_image_count.max(surface_caps.min_image_count));

        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(surface_caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(*present_mode)
            .clipped(true);

        let swapchain = self.swapchain_loader
            .create_swapchain(&create_info, None)
            .map_err(|e| anyhow::anyhow!("Swapchain creation failed: {:?}", e))?;

        let images = self.swapchain_loader
            .get_swapchain_images(swapchain)
            .map_err(|e| anyhow::anyhow!("Get swapchain images failed: {:?}", e))?;

        self.swapchain = swapchain;
        self.swapchain_images = images;
        self.swapchain_format = format.format;
        self.swapchain_extent = extent;

        self.init_staging_buffer(extent)?;

        Ok(())
    }

    unsafe fn init_staging_buffer(&mut self, extent: vk::Extent2D) -> anyhow::Result<()> {
        self.destroy_staging_buffer();

        let buffer_size = (extent.width as vk::DeviceSize)
            * (extent.height as vk::DeviceSize)
            * 4;

        let buffer_ci = vk::BufferCreateInfo::default()
            .size(buffer_size)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let staging_buffer = self.device.create_buffer(&buffer_ci, None)?;

        let mem_requirements = self.device.get_buffer_memory_requirements(staging_buffer);
        let memory_type_index = self.find_memory_type(
            mem_requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_CACHED,
        )?;

        let alloc_ci = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type_index);

        let staging_memory = self.device.allocate_memory(&alloc_ci, None)?;
        self.device.bind_buffer_memory(staging_buffer, staging_memory, 0)?;

        self.staging_buffer = staging_buffer;
        self.staging_memory = staging_memory;
        self.staging_size = buffer_size;

        Ok(())
    }

    unsafe fn find_memory_type(
        &self,
        type_bits: u32,
        property_flags: vk::MemoryPropertyFlags,
    ) -> anyhow::Result<u32> {
        for i in 0..self.memory_properties.memory_type_count {
            if type_bits & (1 << i) != 0 {
                let mem_type = self.memory_properties.memory_types[i as usize];
                if mem_type.property_flags.contains(property_flags) {
                    return Ok(i);
                }
            }
        }
        Err(anyhow::anyhow!("No suitable memory type for {:?}", property_flags))
    }

    /// Captures the current frame from swapchain image at `image_index`.
    ///
    /// # Safety
    /// `image_index` must be a valid index into `self.swapchain_images`.
    pub unsafe fn capture_current_frame(
        &mut self,
        image_index: u32,
    ) -> anyhow::Result<()> {
        if self.staging_buffer == vk::Buffer::null() {
            return Err(anyhow::anyhow!("Staging buffer not initialized"));
        }
        if image_index as usize >= self.swapchain_images.len() {
            return Err(anyhow::anyhow!(
                "Invalid image_index {} (swapchain has {})",
                image_index,
                self.swapchain_images.len()
            ));
        }

        self.device.wait_for_fences(std::slice::from_ref(&self.fence), true, u64::MAX)?;
        self.device.reset_fences(std::slice::from_ref(&self.fence))?;

        self.device.reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())?;
        let begin_ci = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        self.device.begin_command_buffer(self.command_buffer, &begin_ci)?;

        let image = self.swapchain_images[image_index as usize];
        let extent = self.swapchain_extent;

        let barrier_undefined_to_transfer = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );

        self.device.cmd_pipeline_barrier(
            self.command_buffer,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            std::slice::from_ref(&barrier_undefined_to_transfer),
        );

        let region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            });

        self.device.cmd_copy_image_to_buffer(
            self.command_buffer,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            self.staging_buffer,
            std::slice::from_ref(&region),
        );

        let barrier_transfer_to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );

        self.device.cmd_pipeline_barrier(
            self.command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            std::slice::from_ref(&barrier_transfer_to_present),
        );

        self.device.end_command_buffer(self.command_buffer)?;

        let submit_ci = vk::SubmitInfo::default()
            .command_buffers(std::slice::from_ref(&self.command_buffer));

        self.device.queue_submit(
            self.queue,
            std::slice::from_ref(&submit_ci),
            self.fence,
        )?;

        self.device.wait_for_fences(std::slice::from_ref(&self.fence), true, u64::MAX)?;

        let mapped = self.device.map_memory(
            self.staging_memory,
            0,
            self.staging_size,
            vk::MemoryMapFlags::empty(),
        )?;

        let pixel_data = std::slice::from_raw_parts(
            mapped as *const u8,
            self.staging_size as usize,
        );

        self.captured_frame = Some(CapturedFrame {
            width: extent.width,
            height: extent.height,
            pixels: pixel_data.to_vec(),
        });

        self.device.unmap_memory(self.staging_memory);

        Ok(())
    }

    unsafe fn destroy_staging_buffer(&mut self) {
        if self.staging_buffer != vk::Buffer::null() {
            self.device.destroy_buffer(self.staging_buffer, None);
            self.staging_buffer = vk::Buffer::null();
        }
        if self.staging_memory != vk::DeviceMemory::null() {
            self.device.free_memory(self.staging_memory, None);
            self.staging_memory = vk::DeviceMemory::null();
        }
        self.staging_size = 0;
    }

    /// Destroys the current swapchain (if any) and resets state.
    ///
    /// # Safety
    /// The swapchain must not be in use by any in-flight Vulkan commands
    /// other than those waited on by `device_wait_idle()`.
    pub unsafe fn destroy_swapchain(&mut self) {
        if self.swapchain != vk::SwapchainKHR::null() {
            self.device.device_wait_idle().ok();
            self.swapchain_loader.destroy_swapchain(self.swapchain, None);
            self.swapchain = vk::SwapchainKHR::null();
            self.swapchain_images.clear();
            self.swapchain_format = vk::Format::UNDEFINED;
            self.swapchain_extent = vk::Extent2D { width: 0, height: 0 };
            self.destroy_staging_buffer();
        }
    }
}

impl Drop for VulkanHook {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            self.destroy_swapchain();
            if self.fence != vk::Fence::null() {
                self.device.destroy_fence(self.fence, None);
            }
            if self.command_buffer != vk::CommandBuffer::null() {
                self.device.free_command_buffers(
                    self.command_pool,
                    std::slice::from_ref(&self.command_buffer),
                );
            }
            if self.command_pool != vk::CommandPool::null() {
                self.device.destroy_command_pool(self.command_pool, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_frame_default_is_none() {
        if let Ok(hook) = VulkanHook::new() {
            assert!(hook.captured_frame.is_none());
            assert!(!hook.has_swapchain());
        }
    }

    #[test]
    fn hook_selects_graphics_queue() {
        if let Ok(hook) = VulkanHook::new() {
            assert!(hook.queue != vk::Queue::null());
        }
    }

    #[test]
    fn hook_device_name_is_nonempty() {
        if let Ok(hook) = VulkanHook::new() {
            let name = hook.device_name();
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn hook_api_version_is_vulkan_1() {
        if let Ok(hook) = VulkanHook::new() {
            let (major, _minor, _patch) = hook.api_version();
            assert!(major >= 1);
        }
    }

    #[test]
    fn hook_backend_default_is_layer() {
        if let Ok(hook) = VulkanHook::new() {
            assert_eq!(hook.backend, VulkanHookBackend::Layer);
        }
    }

    #[test]
    fn captured_frame_stores_pixels() {
        let frame = CapturedFrame {
            width: 1920,
            height: 1080,
            pixels: vec![0u8; 1920 * 1080 * 4],
        };
        assert_eq!(frame.width, 1920);
        assert_eq!(frame.height, 1080);
        assert_eq!(frame.pixels.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn hook_debug_format() {
        if let Ok(hook) = VulkanHook::new() {
            let debug = format!("{:?}", hook);
            assert!(debug.contains("VulkanHook"));
            assert!(debug.contains("physical_device"));
        }
    }

    #[test]
    fn backend_variants_are_distinct() {
        assert_ne!(VulkanHookBackend::Layer, VulkanHookBackend::Detour);
    }

    #[test]
    fn queue_family_supports_transfer() {
        if let Ok(hook) = VulkanHook::new() {
            assert!(hook.queue_family_supports_transfer());
        }
    }

    #[test]
    fn capture_without_swapchain_fails() {
        if let Ok(mut hook) = VulkanHook::new() {
            let result = unsafe { hook.capture_current_frame(0) };
            assert!(result.is_err());
        }
    }

    #[test]
    fn staging_buffer_not_initialized_capture_fails() {
        if let Ok(mut hook) = VulkanHook::new() {
            let result = unsafe { hook.capture_current_frame(0) };
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Staging buffer"));
        }
    }

    #[test]
    fn invalid_image_index_fails() {
        if let Ok(mut hook) = VulkanHook::new() {
            hook.staging_buffer = ash::vk::Handle::from_raw(1);
            let result = unsafe { hook.capture_current_frame(99) };
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Invalid image_index"));
        }
    }
}