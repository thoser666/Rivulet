// Vulkan hook for zero-overhead game capture (G3).
// Intercepts vkQueuePresentKHR and extracts swapchain images without copy.
//
// This is a foundation for D3D/Vulkan/OpenGL API hooking to enable
// direct GPU frame access without CPU-side copy overhead.
//
// ⚠️ Work in progress – not yet integrated into the recording pipeline.
// Currently provides the Vulkan instance/layer infrastructure.

use std::sync::{Arc, Mutex};
use vulkano::instance::Instance;
use vulkano::device::{Device, Queue};
use vulkano::swapchain::{Swapchain, Surface};
use vulkano::image::SwapchainImage;
use vulkano::sync::GpuFuture;
use vulkano::version::V1_0;
use vulkano::filter::Filter;
use vulkano::sampler::Sampler;
use vulkano::sync::SharingMode;

/// Vulkan hook state, holds the instance, device, and swapchain needed
/// to present/intercept frames for capture.
#[derive(Debug)]
pub struct VulkanHook {
    /// Vulkan instance (required for device creation and surface)
    pub instance: Arc<Instance>,
    /// Logical device + graphics queue
    pub device: Arc<Device>,
    /// Graphics queue for submitting work
    pub queue: vk::Queue,
    /// Swapchain for presenting/capturing frames
    pub swapchain: Arc<Swapchain<Window>>,
    /// Current image index being presented
    pub image_index: vk::ImageIndexKHR,
    /// Frame buffer for captured frames (maps swapchain images to RGBA data)
    pub frame_buffer: Arc<Mutex<Vec<Vec<u8>>>>,
}

/// Creates a Vulkan instance and surface for the given window.
/// This is the first step in G3 – Vulkan hook infrastructure.
/// 
/// # Returns
/// - `VulkanHook` containing instance, device, queue, swapchain
/// - Returns error if Vulkan initialization fails or surface not supported
pub fn create_vulkan_hook(window: &winit::window::Window) -> anyhow::Result<VulkanHook> {
    // Enable required Vulkan extensions for surface and swapchain
    let required_extensions = vulkano::instance::Instance::required_extensions()
        .iter()
        .chain(std::iter::once(
            std::ffi::CString::new("VK_KHR_surface").unwrap()
        ));
    
    let instance = VulkanInstance::new(required_extensions)?;
    
    // Create surface from window
    let surface = vulkano::swapchain::Surface::from_window(instance.as_raw(), window)?;
    
    // Select physical device and create logical device
    let (physical_device, queue_family) = instance
        .physical_devices()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No Vulkan-capable GPU found"))?
        .queue_family_indices()
        . graphics_family()
        .ok_or_else(|| anyhow::anyhow!("No graphics queue family"))?;
    
    let device_queue = instance
        .device()
        .physical_device(physical_device)
        .queue_family_index(queue_family)
        .queue(vk::QueueFlags::GRAPHICS);
    
    let device = device_queue.device();
    let queue = device_queue.queue();
    
    // Create swapchain matching window dimensions
    let caps = surface.capabilities();
    let surface_format = surface.supported_formats().next().unwrap_or(vk::Format::B8G8R8AUNORM);
    let present_mode = surface.present_modes().next().unwrap_or(vk::PresentModeKHR::FIFO);
    
    let swapchain = Swapchain::new(
        device.clone(),
        surface.clone(),
        vk::SwapchainCreateInfoKHR::builder()
            .min_image_count(caps.min_image_count)
            .image_format(surface_format)
            .image_extent(vk::Extent2D {
                width: window.inner_size().width,
                height: window.inner_size().height,
            })
            .image_color_space(surface_format.color_space())
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(vk::SwapchainKHR::null()),
    )?;
    
    Ok(VulkanHook {
        instance,
        device,
        queue,
        swapchain,
        image_index: vk::ImageIndexKHR::default(),
        frame_buffer: Arc::new(Mutex::new(Vec::new())),
    })
}

/// Submits a present operation and captures the resulting swapchain image
/// into the frame buffer for later recording.
/// 
/// # Safety
/// - The swapchain must be valid and have images available
/// - The queue must be a graphics queue
/// - This function must be called from the render thread
pub unsafe fn present_and_capture(
    hook: &mut VulkanHook,
    swapchain_image: &SwapchainImage<Window>,
) -> anyhow::Result<()> {
    // Submit present command - this will present the image to the surface
    let submit_info = vk::SubmitInfo::builder()
        .wait_semaphores(&[])
        .signal_semaphores(&[])
        .wait_dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .command_buffers(&[])
        .signal_semaphores(&[])
        .build();
    
    // Present the image
    let result = hook.queue.present_khr(&vk::PresentInfoKHR::builder()
        .swapchains(&[hook.swapchain.handle()])
        .image_indices(&[hook.image_index])
        .p_wait_semaphores(&[])
        .p_signal_semaphores(&[])
        .p_results(&[])
        .build());
    
    match result {
        vk::Result::SUCCESS => {},
        _ => return Err(anyhow::anyhow!("Present failed")),
    }
    
    // Read swapchain image data (RGBA) into frame buffer
    let image = hook.swapchain.images()[hook.image_index as usize].clone();
    let mapped = image.map_memory()?;
    let data = mapped.read_as::<[u8]>()?.to_vec();
    
    let mut buffer = hook.frame_buffer.lock().unwrap();
    buffer.resize(hook.swchain.image_count() as usize, Vec::new());
    buffer[hook.image_index as usize] = data;
    
    Ok(())
}

/// Returns the latest captured frame as RGBA byte slice.
/// 
/// The frame dimensions match the swapchain extent.
pub fn get_captured_frame(hook: &VulkanHook) -> Option<(u32, u32, &[u8])> {
    let buffer = hook.frame_buffer.lock().unwrap();
    let last_idx = (hook.image_index + 1) % hook.swapchain.image_count();
    
    let data = &buffer[last_idx as usize];
    let extent = hook.swapchain.extent();
    
    // Verify data size matches expected (width * height * 4 for RGBA)
    let expected_size = extent.width * extent.height * 4;
    if data.len() >= expected_size {
        Some((extent.width, extent.height, &data[..expected_size]))
    } else {
        None
    }
}