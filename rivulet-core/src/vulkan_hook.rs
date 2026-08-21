// G3 – Vulkan hook for zero-overhead game capture.
//
// Uses `ash` for raw Vulkan API access.
// Establishes the Vulkan runtime foundation: Entry → Instance → Device → Queue.
// Swapchain interception is the next integration step.
//
// Architecture:
//   - VulkanHook: owns Entry, Instance, Device, Queue, extension loaders
//   - CapturedFrame: frame metadata + RGBA pixel data
//   - VulkanHookBackend: Layer vs Detour strategy enum

use std::ffi::CString;

use ash::{vk, Entry, Instance, Device};

/// Holds all Vulkan resources needed for frame capture.
pub struct VulkanHook {
    pub entry: Entry,
    pub instance: Instance,
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
}

/// A single captured frame with metadata.
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Supported Vulkan hook backends.
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
    /// Creates a new Vulkan hook with its own Entry → Instance → Device → Queue.
    ///
    /// Selects the best available GPU (prefers discrete).
    /// Enables `VK_KHR_surface` + platform surface + `VK_KHR_swapchain`.
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

        let mut ext_names: Vec<*const std::ffi::c_char> = vec![
            vk::KHR_SURFACE_NAME.as_ptr(),
        ];
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

        let dev_ext_names: Vec<*const std::ffi::c_char> =
            vec![vk::KHR_SWAPCHAIN_NAME.as_ptr()];

        let device_ci = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_ci))
            .enabled_extension_names(&dev_ext_names);

        let device = unsafe { instance.create_device(physical_device, &device_ci, None)? };
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);

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
        })
    }

    /// Returns true if a valid swapchain has been created.
    pub fn has_swapchain(&self) -> bool {
        self.swapchain != vk::SwapchainKHR::null()
    }

    /// Returns the name of the selected physical device.
    pub fn device_name(&self) -> String {
        unsafe {
            let props = self
                .instance
                .get_physical_device_properties(self.physical_device);
            let name_bytes: Vec<u8> = props
                .device_name
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8)
                .collect();
            String::from_utf8_lossy(&name_bytes).to_string()
        }
    }

    /// Returns the Vulkan API version as (major, minor, patch).
    pub fn api_version(&self) -> (u32, u32, u32) {
        unsafe {
            let props = self
                .instance
                .get_physical_device_properties(self.physical_device);
            (
                vk::api_version_major(props.api_version),
                vk::api_version_minor(props.api_version),
                vk::api_version_patch(props.api_version),
            )
        }
    }

    /// Creates a swapchain for a given surface and extent.
    ///
    /// In production this would be called when we intercept the target
    /// application's swapchain creation. For standalone capture/testing,
    /// the caller provides the surface.
    ///
    /// # Safety
    /// `surface` must be a valid `VkSurfaceKHR` created from this instance.
    pub unsafe fn create_swapchain(
        &mut self,
        surface: vk::SurfaceKHR,
        extent: vk::Extent2D,
    ) -> anyhow::Result<()> {
        let surface_caps = self
            .surface_loader
            .get_physical_device_surface_capabilities(self.physical_device, surface)
            .map_err(|e| anyhow::anyhow!("Surface capabilities query failed: {:?}", e))?;

        let formats = self
            .surface_loader
            .get_physical_device_surface_formats(self.physical_device, surface)
            .map_err(|e| anyhow::anyhow!("Surface formats query failed: {:?}", e))?;

        let format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .or(formats.first())
            .ok_or_else(|| anyhow::anyhow!("No surface formats available"))?;

        let present_modes = self
            .surface_loader
            .get_physical_device_surface_present_modes(self.physical_device, surface)
            .map_err(|e| anyhow::anyhow!("Surface present modes query failed: {:?}", e))?;

        let present_mode = present_modes
            .iter()
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
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(surface_caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(*present_mode)
            .clipped(true);

        let swapchain = self
            .swapchain_loader
            .create_swapchain(&create_info, None)
            .map_err(|e| anyhow::anyhow!("Swapchain creation failed: {:?}", e))?;

        let images = self
            .swapchain_loader
            .get_swapchain_images(swapchain)
            .map_err(|e| anyhow::anyhow!("Get swapchain images failed: {:?}", e))?;

        self.swapchain = swapchain;
        self.swapchain_images = images;
        self.swapchain_format = format.format;
        self.swapchain_extent = extent;

        Ok(())
    }

    /// Destroys the current swapchain (if any) and resets state.
    ///
    /// # Safety
    /// The swapchain must not be in use by any in-flight Vulkan commands.
    pub unsafe fn destroy_swapchain(&mut self) {
        if self.swapchain != vk::SwapchainKHR::null() {
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
            self.swapchain = vk::SwapchainKHR::null();
            self.swapchain_images.clear();
            self.swapchain_format = vk::Format::UNDEFINED;
            self.swapchain_extent = vk::Extent2D { width: 0, height: 0 };
        }
    }
}

impl Drop for VulkanHook {
    fn drop(&mut self) {
        unsafe {
            self.destroy_swapchain();
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
}
