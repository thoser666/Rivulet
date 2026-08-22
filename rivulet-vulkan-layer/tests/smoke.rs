//! Live smoke test for the Vulkan capture layer (G3).
//!
//! Loads `VK_LAYER_RIVULET_capture` through the real Vulkan loader, drives a
//! minimal instance → device → swapchain → present, and verifies that the
//! layer's `vkQueuePresentKHR` hook forwards the call to the driver and writes
//! a frame into the shared-memory capture channel.
//!
//! Requires a Vulkan-capable GPU and a display, so it is ignored by default.
//! Run locally on Windows with:
//!
//! ```text
//! cargo test -p rivulet-vulkan-layer --test smoke -- --ignored --nocapture
//! ```

#![cfg(target_os = "windows")]

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use rivulet_vulkan_layer::capture_channel::{FrameHeader, SHM_MAGIC, SHM_NAME};
use std::ffi::{CStr, CString};

const LAYER_NAME: &str = "VK_LAYER_RIVULET_capture";

/// Directory holding the built layer DLL + manifest (copied there by
/// `build.rs`). The Vulkan loader resolves the manifest's relative
/// `library_path` against this directory.
fn layer_dir() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    manifest_dir
        .parent()
        .expect("workspace root")
        .join("target")
        .join(profile)
}

fn create_instance(entry: &ash::Entry) -> ash::Instance {
    let app_name = CString::new("Rivulet layer smoke test").unwrap();
    let engine_name = CString::new("Rivulet").unwrap();
    let layer_name = CString::new(LAYER_NAME).unwrap();
    let layer_name_ptr = layer_name.as_ptr();

    let app_info = vk::ApplicationInfo::default()
        .application_name(app_name.as_c_str())
        .application_version(vk::make_api_version(0, 1, 0, 0))
        .engine_name(engine_name.as_c_str())
        .engine_version(vk::make_api_version(0, 1, 0, 0))
        .api_version(vk::make_api_version(0, 1, 3, 0));

    let ext_names = [
        vk::KHR_SURFACE_NAME.as_ptr(),
        vk::KHR_WIN32_SURFACE_NAME.as_ptr(),
    ];

    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&ext_names)
        .enabled_layer_names(std::slice::from_ref(&layer_name_ptr));

    unsafe {
        entry
            .create_instance(&create_info, None)
            .expect("create instance")
    }
}

/// Pick the discrete GPU (fallback: integrated, then first device).
fn pick_physical_device(instance: &ash::Instance) -> vk::PhysicalDevice {
    let devices = unsafe {
        instance
            .enumerate_physical_devices()
            .expect("enumerate devices")
    };
    assert!(!devices.is_empty(), "no Vulkan physical device");
    *devices
        .iter()
        .min_by_key(|pd| {
            let props = unsafe { instance.get_physical_device_properties(**pd) };
            match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 0u32,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                _ => 2,
            }
        })
        .unwrap()
}

fn graphics_queue_family(instance: &ash::Instance, pd: vk::PhysicalDevice) -> u32 {
    let families = unsafe { instance.get_physical_device_queue_family_properties(pd) };
    families
        .iter()
        .position(|qf| qf.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .expect("no graphics queue family") as u32
}

fn create_device(
    instance: &ash::Instance,
    pd: vk::PhysicalDevice,
    queue_family: u32,
) -> (ash::Device, vk::Queue) {
    let priorities = [1.0f32];
    let queue_ci = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family)
        .queue_priorities(&priorities);
    let ext_names = [vk::KHR_SWAPCHAIN_NAME.as_ptr()];
    let device_ci = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_ci))
        .enabled_extension_names(&ext_names);
    let device = unsafe {
        instance
            .create_device(pd, &device_ci, None)
            .expect("create device")
    };
    let queue = unsafe { device.get_device_queue(queue_family, 0) };
    (device, queue)
}

/// Read the 32-byte frame header from the named shared-memory region, if the
/// layer has created and written it yet.
fn read_shm_header() -> Option<FrameHeader> {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::memoryapi::{MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ};
    use winapi::um::winbase::OpenFileMappingA;

    let name = CString::new(SHM_NAME).ok()?;
    unsafe {
        let handle = OpenFileMappingA(FILE_MAP_READ, 0, name.as_ptr());
        if handle.is_null() {
            return None;
        }
        let ptr = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        CloseHandle(handle);
        if ptr.is_null() {
            return None;
        }
        let header = std::slice::from_raw_parts(ptr as *const u8, FrameHeader::SIZE);
        let mut buf = [0u8; FrameHeader::SIZE];
        buf.copy_from_slice(header);
        UnmapViewOfFile(ptr);
        FrameHeader::from_bytes(&buf)
    }
}

#[test]
#[ignore = "requires a Vulkan-capable GPU and a display"]
// `EventLoop::create_window` is deprecated in winit 0.30 in favour of the
// async `ApplicationHandler` flow; the sync path still works and keeps this
// smoke test linear.
#[allow(deprecated)]
fn layer_loads_and_presents_through_real_vulkan() {
    // 1. Enable the layer through the loader environment.
    let dir = layer_dir();
    std::env::set_var("VK_LAYER_PATH", &dir);
    std::env::set_var("VK_INSTANCE_LAYERS", LAYER_NAME);
    eprintln!("VK_LAYER_PATH={}", dir.display());

    // 2. The loader must discover the layer (manifest + DLL colocated).
    let entry = unsafe { ash::Entry::load().expect("load Vulkan loader") };
    let layers = unsafe {
        entry
            .enumerate_instance_layer_properties()
            .expect("enumerate instance layers")
    };
    let found = layers.iter().any(|l| {
        let name = unsafe { CStr::from_ptr(l.layer_name.as_ptr()) };
        name.to_string_lossy() == LAYER_NAME
    });
    assert!(
        found,
        "layer {LAYER_NAME} not discoverable — is the manifest colocated with the DLL?"
    );
    eprintln!("layer discoverable: {LAYER_NAME}");

    // 3. Real instance + device (exercises the layer's vkCreateInstance /
    //    vkCreateDevice hooks).
    let instance = create_instance(&entry);
    let pd = pick_physical_device(&instance);
    let queue_family = graphics_queue_family(&instance, pd);
    let (device, queue) = create_device(&instance, pd, queue_family);
    eprintln!("instance + device created via the layer");

    // 4. A real window + Win32 surface.
    let event_loop = winit::event_loop::EventLoop::new().expect("create event loop");
    let window_attrs = winit::window::Window::default_attributes()
        .with_title("Rivulet Vulkan layer smoke test")
        .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
    let window = event_loop
        .create_window(window_attrs)
        .expect("create window");
    window.set_visible(true);

    let display_handle = window.display_handle().expect("display handle").as_raw();
    let window_handle = window.window_handle().expect("window handle").as_raw();
    let surface = unsafe {
        ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)
            .expect("create surface")
    };

    // The graphics queue must be able to present to this surface.
    let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
    let present_supported = unsafe {
        surface_loader
            .get_physical_device_surface_support(pd, queue_family, surface)
            .expect("query surface support")
    };
    assert!(present_supported, "graphics queue family cannot present");

    // 5. Swapchain (the layer's vkCreateSwapchainKHR hook initializes its
    //    capture pipeline here).
    let caps = unsafe {
        surface_loader
            .get_physical_device_surface_capabilities(pd, surface)
            .expect("surface capabilities")
    };
    let formats = unsafe {
        surface_loader
            .get_physical_device_surface_formats(pd, surface)
            .expect("surface formats")
    };
    let present_modes = unsafe {
        surface_loader
            .get_physical_device_surface_present_modes(pd, surface)
            .expect("surface present modes")
    };
    let format = formats
        .iter()
        .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
        .or_else(|| formats.first())
        .expect("no surface format");
    let present_mode = present_modes
        .iter()
        .find(|&&m| m == vk::PresentModeKHR::MAILBOX)
        .copied()
        .unwrap_or(vk::PresentModeKHR::FIFO);
    let extent = if caps.current_extent.width == u32::MAX || caps.current_extent.height == u32::MAX
    {
        vk::Extent2D {
            width: 800,
            height: 600,
        }
    } else {
        caps.current_extent
    };

    let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);
    let image_count =
        (caps.min_image_count + 1).min(caps.max_image_count.max(caps.min_image_count));
    let swapchain_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(image_count)
        .image_format(format.format)
        .image_color_space(format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC,
        )
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(caps.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(present_mode)
        .clipped(true);
    let swapchain = unsafe {
        swapchain_loader
            .create_swapchain(&swapchain_info, None)
            .expect("create swapchain")
    };

    // 6. Present a frame through the layer's vkQueuePresentKHR hook.
    let (image_index, _suboptimal) = unsafe {
        swapchain_loader
            .acquire_next_image(
                swapchain,
                u64::MAX,
                vk::Semaphore::null(),
                vk::Fence::null(),
            )
            .expect("acquire next image")
    };
    let present_info = vk::PresentInfoKHR::default()
        .swapchains(std::slice::from_ref(&swapchain))
        .image_indices(std::slice::from_ref(&image_index));
    unsafe {
        swapchain_loader
            .queue_present(queue, &present_info)
            .expect("queue present");
    }
    eprintln!("present passed through the layer");

    // 7. The layer must have written a valid frame header to shared memory.
    let mut header = None;
    for _ in 0..20 {
        if let Some(h) = read_shm_header() {
            header = Some(h);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let header = header.expect("layer did not write a frame to shared memory");
    assert_eq!(header.magic, SHM_MAGIC, "bad frame header magic");
    assert!(
        header.width > 0 && header.height > 0,
        "bad frame dimensions"
    );
    assert_eq!(
        header.data_size,
        header.width * header.height * 4,
        "frame data_size mismatch"
    );
    eprintln!(
        "layer captured a frame: {}x{} ({} bytes)",
        header.width, header.height, header.data_size
    );

    // Cleanup (the layer hooks vkDestroySwapchainKHR / vkDestroyDevice).
    unsafe {
        swapchain_loader.destroy_swapchain(swapchain, None);
        surface_loader.destroy_surface(surface, None);
    }
    drop(device);
    drop(instance);
    drop(window);
    drop(event_loop);
}
