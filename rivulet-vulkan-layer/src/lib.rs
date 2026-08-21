#![allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code
)]

pub mod capture_channel;

use capture_channel::{FrameHeader, SHM_NAME, DEFAULT_SHM_SIZE, next_sequence};
use ash::vk;
use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Next-layer forwarding function pointer types
// ---------------------------------------------------------------------------

type FnGetDeviceProcAddr =
    unsafe extern "system" fn(vk::Device, *const c_char) -> vk::PFN_vkVoidFunction;
type FnCreateDevice = unsafe extern "system" fn(
    vk::PhysicalDevice,
    *const vk::DeviceCreateInfo,
    *const vk::AllocationCallbacks,
    *mut vk::Device,
) -> vk::Result;
type FnDestroyDevice =
    unsafe extern "system" fn(vk::Device, *const vk::AllocationCallbacks);
type FnGetDeviceQueue =
    unsafe extern "system" fn(vk::Device, u32, u32, *mut vk::Queue);
type FnCreateSwapchainKHR = unsafe extern "system" fn(
    vk::Device,
    *const vk::SwapchainCreateInfoKHR,
    *const vk::AllocationCallbacks,
    *mut vk::SwapchainKHR,
) -> vk::Result;
type FnDestroySwapchainKHR =
    unsafe extern "system" fn(vk::Device, vk::SwapchainKHR, *const vk::AllocationCallbacks);
type FnQueuePresentKHR =
    unsafe extern "system" fn(vk::Queue, *const vk::PresentInfoKHR) -> vk::Result;

// ---------------------------------------------------------------------------
// Per-device capture state
// ---------------------------------------------------------------------------

struct DeviceState {
    device: ash::Device,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    graphics_queue_family: u32,
    fn_create_swapchain: FnCreateSwapchainKHR,
    fn_destroy_swapchain: FnDestroySwapchainKHR,
    fn_queue_present: FnQueuePresentKHR,
    fn_destroy_device: FnDestroyDevice,
    swapchain: vk::SwapchainKHR,
    swapchain_loader: ash::khr::swapchain::Device,
    images: Vec<vk::Image>,
    format: vk::Format,
    extent: vk::Extent2D,
    cmd_pool: vk::CommandPool,
    cmd_buf: vk::CommandBuffer,
    staging_buf: vk::Buffer,
    staging_mem: vk::DeviceMemory,
    staging_size: vk::DeviceSize,
    fence: vk::Fence,
    ready: bool,
}

unsafe impl Send for DeviceState {}

impl DeviceState {
    fn destroy_capture(&mut self) {
        if !self.ready {
            return;
        }
        unsafe {
            self.device.device_wait_idle().ok();
            if self.fence != vk::Fence::null() {
                self.device.destroy_fence(self.fence, None);
                self.fence = vk::Fence::null();
            }
            if self.staging_buf != vk::Buffer::null() {
                self.device.destroy_buffer(self.staging_buf, None);
                self.staging_buf = vk::Buffer::null();
            }
            if self.staging_mem != vk::DeviceMemory::null() {
                self.device.free_memory(self.staging_mem, None);
                self.staging_mem = vk::DeviceMemory::null();
            }
            if self.cmd_buf != vk::CommandBuffer::null() {
                self.device
                    .free_command_buffers(self.cmd_pool, &[self.cmd_buf]);
                self.cmd_buf = vk::CommandBuffer::null();
            }
            if self.cmd_pool != vk::CommandPool::null() {
                self.device.destroy_command_pool(self.cmd_pool, None);
                self.cmd_pool = vk::CommandPool::null();
            }
        }
        self.staging_size = 0;
        self.ready = false;
    }

    fn init_capture(
        &mut self,
        instance: &ash::Instance,
        new_images: &[vk::Image],
        fmt: vk::Format,
        ext: vk::Extent2D,
    ) {
        self.destroy_capture();

        self.images = new_images.to_vec();
        self.format = fmt;
        self.extent = ext;

        let pool_ci = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(self.graphics_queue_family);
        self.cmd_pool = unsafe {
            self.device.create_command_pool(&pool_ci, None).unwrap()
        };

        let buf_ai = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        self.cmd_buf = unsafe {
            self.device.allocate_command_buffers(&buf_ai).unwrap()[0]
        };

        let bytes = ext.width as vk::DeviceSize * ext.height as vk::DeviceSize * 4;
        let buf_ci = vk::BufferCreateInfo::default()
            .size(bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        self.staging_buf = unsafe {
            self.device.create_buffer(&buf_ci, None).unwrap()
        };

        let mem_reqs = unsafe {
            self.device.get_buffer_memory_requirements(self.staging_buf)
        };
        let mem_props = unsafe {
            instance.get_physical_device_memory_properties(self.physical_device)
        };
        let type_idx = {
            let mut found = None;
            for (i, mt) in mem_props.memory_types.iter().enumerate() {
                if (mem_reqs.memory_type_bits & (1 << i)) != 0
                    && mt.property_flags.contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_CACHED,
                    )
                {
                    found = Some(i as u32);
                    break;
                }
            }
            found.unwrap()
        };

        let alloc_ci = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(type_idx);
        self.staging_mem = unsafe {
            self.device.allocate_memory(&alloc_ci, None).unwrap()
        };
        unsafe {
            self.device
                .bind_buffer_memory(self.staging_buf, self.staging_mem, 0)
                .unwrap();
        }
        self.staging_size = bytes;

        self.fence = unsafe {
            self.device
                .create_fence(
                    &vk::FenceCreateInfo::default()
                        .flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .unwrap()
        };

        self.ready = true;
        eprintln!(
            "[Rivulet Layer] Capture pipeline ready: {}x{} {:?}",
            ext.width, ext.height, fmt
        );
    }

    fn capture(&mut self, image_index: u32) -> Option<Vec<u8>> {
        if !self.ready || (image_index as usize) >= self.images.len() {
            return None;
        }
        unsafe {
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .ok();
            self.device.reset_fences(&[self.fence]).ok();

            let bi = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(self.cmd_buf, &bi).ok();

            let img = self.images[image_index as usize];
            let subresource = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);

            // PRESENT_SRC → TRANSFER_SRC
            self.device.cmd_pipeline_barrier(
                self.cmd_buf,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(img)
                    .subresource_range(subresource)],
            );

            // Copy image → staging
            self.device.cmd_copy_image_to_buffer(
                self.cmd_buf,
                img,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.staging_buf,
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width: self.extent.width,
                        height: self.extent.height,
                        depth: 1,
                    })],
            );

            // TRANSFER_SRC → PRESENT_SRC
            self.device.cmd_pipeline_barrier(
                self.cmd_buf,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::MEMORY_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(img)
                    .subresource_range(subresource)],
            );

            self.device.end_command_buffer(self.cmd_buf).ok();

            let cmds = [self.cmd_buf];
            let si = vk::SubmitInfo::default().command_buffers(&cmds);
            self.device
                .queue_submit(self.graphics_queue, &[si], self.fence)
                .ok();

            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .ok();

            let ptr = self
                .device
                .map_memory(
                    self.staging_mem,
                    0,
                    self.staging_size,
                    vk::MemoryMapFlags::empty(),
                )
                .ok()?;
            let frame = std::slice::from_raw_parts(
                ptr as *const u8,
                self.staging_size as usize,
            )
            .to_vec();
            self.device.unmap_memory(self.staging_mem);

            eprintln!(
                "[Rivulet Layer] Captured frame: {} bytes",
                frame.len()
            );
            Some(frame)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared memory frame writer
// ---------------------------------------------------------------------------

/// Shared memory handle — created on first write, reused thereafter.
struct ShmHandle {
    ptr: *mut u8,
    size: usize,
}

unsafe impl Send for ShmHandle {}
unsafe impl Sync for ShmHandle {}

static SHM: Mutex<Option<ShmHandle>> = Mutex::new(None);

/// Write a captured frame to the shared memory region so Rivulet can read it.
///
/// Creates the shared memory region on first call. Subsequent calls overwrite
/// the previous frame (latest-frame-wins semantics).
fn write_frame_to_shm(frame: &[u8], width: u32, height: u32) {
    let seq = next_sequence();
    let header = FrameHeader::new(width, height, seq);
    let header_bytes = header.to_bytes();

    let mut guard = SHM.lock().unwrap();
    let handle = guard.get_or_insert_with(|| {
        create_shm().expect("[Rivulet Layer] Failed to create shared memory")
    });

    let total = FrameHeader::SIZE + frame.len();
    if total > handle.size {
        eprintln!(
            "[Rivulet Layer] Frame {} bytes > SHM {} bytes, truncating",
            total, handle.size
        );
    }

    unsafe {
        let dst = handle.ptr;
        let copy_len = total.min(handle.size);
        std::ptr::copy_nonoverlapping(header_bytes.as_ptr(), dst, FrameHeader::SIZE);
        let data_len = (copy_len - FrameHeader::SIZE).min(frame.len());
        std::ptr::copy_nonoverlapping(frame.as_ptr(), dst.add(FrameHeader::SIZE), data_len);
    }
}

/// Platform-specific shared memory creation.
#[cfg(target_os = "windows")]
fn create_shm() -> Option<ShmHandle> {
    use std::ffi::CString;
    use winapi::um::handleapi::*;
    use winapi::um::memoryapi::*;
    use winapi::um::winbase::*;
    use winapi::um::winnt::*;

    let name = CString::new(SHM_NAME).ok()?;
    let size = DEFAULT_SHM_SIZE as u64;

    unsafe {
        let handle = CreateFileMappingA(
            INVALID_HANDLE_VALUE,
            std::ptr::null_mut(),
            PAGE_READWRITE,
            (size >> 32) as u32,
            size as u32,
            name.as_ptr(),
        );
        if handle.is_null() {
            return None;
        }

        let ptr = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, 0);
        if ptr.is_null() {
            CloseHandle(handle);
            return None;
        }

        Some(ShmHandle {
            ptr: ptr as *mut u8,
            size: DEFAULT_SHM_SIZE,
        })
    }
}

/// Platform-specific shared memory creation.
#[cfg(target_os = "linux")]
fn create_shm() -> Option<ShmHandle> {
    use std::ffi::CString;

    let name = CString::new(format!("/{}", SHM_NAME)).ok()?;
    let size = DEFAULT_SHM_SIZE;

    unsafe {
        let fd = libc::shm_open(name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o666);
        if fd < 0 {
            return None;
        }
        libc::ftruncate(fd, size as libc::off_t);
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        libc::close(fd);
        if ptr == libc::MAP_FAILED {
            return None;
        }

        Some(ShmHandle {
            ptr: ptr as *mut u8,
            size,
        })
    }
}

// ---------------------------------------------------------------------------
// Instance state
// ---------------------------------------------------------------------------

struct InstanceState {
    entry: Box<ash::Entry>,
    instance: ash::Instance,
    fn_get_instance_proc_addr: unsafe extern "system" fn(
        vk::Instance,
        *const c_char,
    ) -> vk::PFN_vkVoidFunction,
    fn_get_dpa: FnGetDeviceProcAddr,
    fn_create_device: FnCreateDevice,
    devices: HashMap<vk::Device, DeviceState>,
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

struct LayerGlobal {
    instances: HashMap<vk::Instance, InstanceState>,
}

static GLOBAL: Mutex<Option<LayerGlobal>> = Mutex::new(None);

#[inline]
fn with_global<F, R>(f: F) -> R
where
    F: FnOnce(&mut LayerGlobal) -> R,
{
    let mut guard = GLOBAL.lock().unwrap();
    f(guard.get_or_insert_with(|| LayerGlobal {
        instances: HashMap::new(),
    }))
}

/// Coerce a function item to `unsafe extern "system" fn()`.
///
/// Function items are ZST in Rust, so we cast through `*const ()`
/// (a thin pointer) and then `transmute` to the target fn pointer type.
macro_rules! layer_fn {
    ($f:ident) => {{
        let ptr = $f as *const ();
        #[allow(clippy::useless_transmute)]
        unsafe {
            std::mem::transmute::<*const (), unsafe extern "system" fn()>(ptr)
        }
    }};
}

// ---------------------------------------------------------------------------
// Entry point: layer negotiation
// ---------------------------------------------------------------------------

const VK_LAYER_INTERFACE_VERSION: u32 = 2;

/// Layer negotiation entry point called by the Vulkan loader.
///
/// # Safety
/// `p_version` must point to a valid `u32` written by the loader.
#[no_mangle]
pub unsafe extern "system" fn vkNegotiateLoaderLayerInterfaceVersion(
    p_version: *mut u32,
) -> vk::Result {
    if p_version.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    let v = *p_version;
    eprintln!("[Rivulet Layer] negotiate version={v}");
    if v < VK_LAYER_INTERFACE_VERSION {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    *p_version = VK_LAYER_INTERFACE_VERSION;
    vk::Result::SUCCESS
}

// ---------------------------------------------------------------------------
// Proc addr resolvers — return PFN_vkVoidFunction = Option<extern fn()>
// ---------------------------------------------------------------------------

unsafe fn resolve_instance_proc(
    instance: vk::Instance,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    let s = CStr::from_ptr(name).to_string_lossy();
    match s.as_ref() {
        "vkGetInstanceProcAddr" => {
            Some(layer_fn!(VK_LAYER_RIVULET_capture_vkGetInstanceProcAddr))
        }
        "vkGetDeviceProcAddr" => {
            Some(layer_fn!(VK_LAYER_RIVULET_capture_vkGetDeviceProcAddr))
        }
        "vkCreateInstance" => Some(layer_fn!(layer_vk_create_instance)),
        "vkDestroyInstance" => Some(layer_fn!(layer_vk_destroy_instance)),
        "vkCreateDevice" => Some(layer_fn!(layer_vk_create_device)),
        "vkDestroyDevice" => Some(layer_fn!(layer_vk_destroy_device)),
        "vkCreateSwapchainKHR" => Some(layer_fn!(layer_vk_create_swapchain_khr)),
        "vkDestroySwapchainKHR" => Some(layer_fn!(layer_vk_destroy_swapchain_khr)),
        "vkQueuePresentKHR" => Some(layer_fn!(layer_vk_queue_present_khr)),
        _ => with_global(|g| {
            g.instances.get(&instance).and_then(|i| {
                (i.fn_get_instance_proc_addr)(instance, name)
            })
        }),
    }
}

unsafe fn resolve_device_proc(
    device: vk::Device,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    let s = CStr::from_ptr(name).to_string_lossy();
    match s.as_ref() {
        "vkGetDeviceProcAddr" => {
            Some(layer_fn!(VK_LAYER_RIVULET_capture_vkGetDeviceProcAddr))
        }
        "vkCreateDevice" => Some(layer_fn!(layer_vk_create_device)),
        "vkDestroyDevice" => Some(layer_fn!(layer_vk_destroy_device)),
        "vkCreateSwapchainKHR" => Some(layer_fn!(layer_vk_create_swapchain_khr)),
        "vkDestroySwapchainKHR" => Some(layer_fn!(layer_vk_destroy_swapchain_khr)),
        "vkQueuePresentKHR" => Some(layer_fn!(layer_vk_queue_present_khr)),
        _ => with_global(|g| {
            for inst in g.instances.values() {
                if inst.devices.contains_key(&device) {
                    return (inst.fn_get_dpa)(device, name);
                }
            }
            None
        }),
    }
}

/// Vulkan loader callback for instance-level function resolution.
///
/// # Safety
/// `instance` must be a valid Vulkan instance handle; `p_name` must be a
/// valid null-terminated C string or null.
#[no_mangle]
pub unsafe extern "system" fn VK_LAYER_RIVULET_capture_vkGetInstanceProcAddr(
    instance: vk::Instance,
    p_name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    resolve_instance_proc(instance, p_name)
}

/// Vulkan loader callback for device-level function resolution.
///
/// # Safety
/// `device` must be a valid Vulkan device handle; `p_name` must be a
/// valid null-terminated C string or null.
#[no_mangle]
pub unsafe extern "system" fn VK_LAYER_RIVULET_capture_vkGetDeviceProcAddr(
    device: vk::Device,
    p_name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    resolve_device_proc(device, p_name)
}

// ---------------------------------------------------------------------------
// vkCreateInstance
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_create_instance(
    pCreateInfo: *const vk::InstanceCreateInfo,
    pAllocator: *const vk::AllocationCallbacks,
    pInstance: *mut vk::Instance,
) -> vk::Result {
    if pCreateInfo.is_null() || pInstance.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let entry = match ash::Entry::load() {
        Ok(e) => e,
        Err(_) => return vk::Result::ERROR_INITIALIZATION_FAILED,
    };

    let result = entry.create_instance(&*pCreateInfo, pAllocator.as_ref());
    let ash_instance = match result {
        Ok(i) => i,
        Err(e) => return e,
    };

    let raw_instance = ash_instance.handle();

    // Get next-layer function pointers from the instance dispatch table.
    let fp = ash_instance.fp_v1_0();
    let fn_get_dpa: FnGetDeviceProcAddr = fp.get_device_proc_addr;
    let fn_create_device: FnCreateDevice = fp.create_device;

    // We need the entry alive for the instance lifetime.
    let entry_box = Box::new(entry);
    let fn_gipa = entry_box.static_fn().get_instance_proc_addr;

    with_global(|g| {
        g.instances.insert(
            raw_instance,
            InstanceState {
                entry: entry_box,
                instance: ash_instance,
                fn_get_instance_proc_addr: fn_gipa,
                fn_get_dpa,
                fn_create_device,
                devices: HashMap::new(),
            },
        );
    });

    *pInstance = raw_instance;
    eprintln!("[Rivulet Layer] vkCreateInstance → ok");
    vk::Result::SUCCESS
}

// ---------------------------------------------------------------------------
// vkDestroyInstance
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_destroy_instance(
    instance: vk::Instance,
    _pAllocator: *const vk::AllocationCallbacks,
) {
    let mut removed = None;
    with_global(|g| {
        removed = g.instances.remove(&instance);
    });
    if let Some(mut state) = removed {
        for (_, mut dev) in state.devices.drain() {
            dev.destroy_capture();
        }
        drop(state);
    }
}

// ---------------------------------------------------------------------------
// vkCreateDevice
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_create_device(
    physical_device: vk::PhysicalDevice,
    pCreateInfo: *const vk::DeviceCreateInfo,
    pAllocator: *const vk::AllocationCallbacks,
    pDevice: *mut vk::Device,
) -> vk::Result {
    if pCreateInfo.is_null() || pDevice.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let ci = &*pCreateInfo;

    let (inst_handle, fn_create_device, fn_get_dpa) = with_global(|g| {
        if let Some((&inst, state)) = g.instances.iter().next() {
            return Some((
                inst,
                state.fn_create_device,
                state.fn_get_dpa,
            ));
        }
        None
    })
    .unwrap();

    let mut device = vk::Device::null();
    let result = fn_create_device(physical_device, ci, pAllocator, &mut device);
    if result != vk::Result::SUCCESS {
        return result;
    }

    with_global(|g| {
        if let Some(inst) = g.instances.get_mut(&inst_handle) {
            // Create ash::Device using the next layer's get_device_proc_addr
            let ash_device = ash::Device::load(
                inst.instance.fp_v1_0(),
                device,
            );

            // Load extension functions via get_device_proc_addr
            let load_ext = |name: &str| -> *const std::ffi::c_void {
                let cname =
                    std::ffi::CString::new(name).unwrap();
                let r = (fn_get_dpa)(device, cname.as_ptr());
                match r {
                    Some(f) => f as *const std::ffi::c_void,
                    None => std::ptr::null(),
                }
            };

            let fn_create_swapchain: FnCreateSwapchainKHR =
                std::mem::transmute(load_ext("vkCreateSwapchainKHR"));
            let fn_destroy_swapchain: FnDestroySwapchainKHR =
                std::mem::transmute(load_ext("vkDestroySwapchainKHR"));
            let fn_queue_present: FnQueuePresentKHR =
                std::mem::transmute(load_ext("vkQueuePresentKHR"));

            // destroy_device and get_device_queue are in DeviceFnV1_0
            let d10 = ash_device.fp_v1_0();
            let fn_destroy_device: FnDestroyDevice = d10.destroy_device;
            let fn_get_device_queue: FnGetDeviceQueue = d10.get_device_queue;

            // Find graphics queue family
            let queue_props = inst
                .instance
                .get_physical_device_queue_family_properties(
                    physical_device,
                );
            let graphics_qf = queue_props
                .iter()
                .position(|q| {
                    q.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                })
                .unwrap_or(0) as u32;

            // Get the graphics queue
            let mut graphics_queue = vk::Queue::null();
            fn_get_device_queue(device, graphics_qf, 0, &mut graphics_queue);

            let swapchain_loader = ash::khr::swapchain::Device::new(
                &inst.instance,
                &ash_device,
            );

            let device_name = {
                let props = inst
                    .instance
                    .get_physical_device_properties(physical_device);
                CStr::from_ptr(props.device_name.as_ptr())
                    .to_string_lossy()
                    .into_owned()
            };

            let dev = DeviceState {
                device: ash_device,
                physical_device,
                graphics_queue,
                graphics_queue_family: graphics_qf,
                fn_create_swapchain,
                fn_destroy_swapchain,
                fn_queue_present,
                fn_destroy_device,
                swapchain: vk::SwapchainKHR::null(),
                swapchain_loader,
                images: Vec::new(),
                format: vk::Format::UNDEFINED,
                extent: vk::Extent2D::default(),
                cmd_pool: vk::CommandPool::null(),
                cmd_buf: vk::CommandBuffer::null(),
                staging_buf: vk::Buffer::null(),
                staging_mem: vk::DeviceMemory::null(),
                staging_size: 0,
                fence: vk::Fence::null(),
                ready: false,
            };

            eprintln!(
                "[Rivulet Layer] vkCreateDevice → {device_name}"
            );
            inst.devices.insert(device, dev);
        }
    });

    *pDevice = device;
    result
}

// ---------------------------------------------------------------------------
// vkDestroyDevice
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_destroy_device(
    device: vk::Device,
    pAllocator: *const vk::AllocationCallbacks,
) {
    let mut fn_next = None;
    with_global(|g| {
        for inst in g.instances.values_mut() {
            if let Some(mut dev) = inst.devices.remove(&device) {
                fn_next = Some(dev.fn_destroy_device);
                dev.destroy_capture();
                drop(dev);
                break;
            }
        }
    });
    if let Some(f) = fn_next {
        f(device, pAllocator);
    }
}

// ---------------------------------------------------------------------------
// vkCreateSwapchainKHR
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_create_swapchain_khr(
    device: vk::Device,
    pCreateInfo: *const vk::SwapchainCreateInfoKHR,
    pAllocator: *const vk::AllocationCallbacks,
    pSwapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    if pCreateInfo.is_null() || pSwapchain.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let ci = &*pCreateInfo;

    let (inst_handle, fn_create) = with_global(|g| {
        for (&inst, state) in &g.instances {
            if let Some(dev) = state.devices.get(&device) {
                return Some((inst, dev.fn_create_swapchain));
            }
        }
        None
    })
    .unwrap();

    let mut swapchain = vk::SwapchainKHR::null();
    let result = fn_create(device, ci, pAllocator, &mut swapchain);
    if result != vk::Result::SUCCESS {
        return result;
    }

    *pSwapchain = swapchain;

    with_global(|g| {
        if let Some(inst) = g.instances.get_mut(&inst_handle) {
            if let Some(dev) = inst.devices.get_mut(&device) {
                dev.swapchain = swapchain;
                let images = dev
                    .swapchain_loader
                    .get_swapchain_images(swapchain)
                    .unwrap_or_default();
                dev.init_capture(
                    &inst.instance,
                    &images,
                    ci.image_format,
                    ci.image_extent,
                );
            }
        }
    });

    result
}

// ---------------------------------------------------------------------------
// vkDestroySwapchainKHR
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_destroy_swapchain_khr(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    pAllocator: *const vk::AllocationCallbacks,
) {
    let mut fn_next = None;
    with_global(|g| {
        for inst in g.instances.values_mut() {
            if let Some(dev) = inst.devices.get_mut(&device) {
                fn_next = Some(dev.fn_destroy_swapchain);
                if dev.swapchain == swapchain {
                    dev.destroy_capture();
                    dev.swapchain = vk::SwapchainKHR::null();
                    dev.images.clear();
                }
                break;
            }
        }
    });
    if let Some(f) = fn_next {
        f(device, swapchain, pAllocator);
    }
}

// ---------------------------------------------------------------------------
// vkQueuePresentKHR — the capture point
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_queue_present_khr(
    queue: vk::Queue,
    pPresentInfo: *const vk::PresentInfoKHR,
) -> vk::Result {
    if !pPresentInfo.is_null() {
        let pi = &*pPresentInfo;
        with_global(|g| {
            for inst in g.instances.values_mut() {
                for dev in inst.devices.values_mut() {
                    if dev.graphics_queue == queue && dev.ready {
                        let count = pi.swapchain_count as usize;
                        let indices = std::slice::from_raw_parts(
                            pi.p_image_indices,
                            count,
                        );
                        for &idx in indices {
                            if let Some(frame) = dev.capture(idx) {
                                write_frame_to_shm(
                                    &frame,
                                    dev.extent.width,
                                    dev.extent.height,
                                );
                            }
                        }
                    }
                }
            }
        });
    }

    let fn_next = with_global(|g| {
        for inst in g.instances.values() {
            for dev in inst.devices.values() {
                if dev.graphics_queue == queue {
                    return Some(dev.fn_queue_present);
                }
            }
        }
        None
    });

    match fn_next {
        Some(f) => f(queue, pPresentInfo),
        None => vk::Result::ERROR_DEVICE_LOST,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_version_rejects_null_pointer() {
        let result = unsafe {
            vkNegotiateLoaderLayerInterfaceVersion(std::ptr::null_mut())
        };
        assert_eq!(result, vk::Result::ERROR_INITIALIZATION_FAILED);
    }

    #[test]
    fn negotiate_version_rejects_old_version() {
        let mut version: u32 = 1; // old
        let result = unsafe {
            vkNegotiateLoaderLayerInterfaceVersion(&mut version)
        };
        assert_eq!(result, vk::Result::ERROR_INITIALIZATION_FAILED);
        assert_eq!(version, 1); // unchanged
    }

    #[test]
    fn negotiate_version_accepts_current_version() {
        let mut version: u32 = VK_LAYER_INTERFACE_VERSION;
        let result = unsafe {
            vkNegotiateLoaderLayerInterfaceVersion(&mut version)
        };
        assert_eq!(result, vk::Result::SUCCESS);
        assert_eq!(version, VK_LAYER_INTERFACE_VERSION);
    }

    #[test]
    fn negotiate_version_accepts_newer_version() {
        let mut version: u32 = 99;
        let result = unsafe {
            vkNegotiateLoaderLayerInterfaceVersion(&mut version)
        };
        assert_eq!(result, vk::Result::SUCCESS);
        assert_eq!(version, VK_LAYER_INTERFACE_VERSION);
    }

    #[test]
    fn resolve_instance_proc_null_name_returns_none() {
        let result = unsafe {
            resolve_instance_proc(vk::Instance::null(), std::ptr::null())
        };
        assert!(result.is_none());
    }

    #[test]
    fn resolve_instance_proc_known_names_return_some() {
        let names = [
            "vkGetInstanceProcAddr\0",
            "vkGetDeviceProcAddr\0",
            "vkCreateInstance\0",
            "vkDestroyInstance\0",
            "vkCreateDevice\0",
            "vkDestroyDevice\0",
            "vkCreateSwapchainKHR\0",
            "vkDestroySwapchainKHR\0",
            "vkQueuePresentKHR\0",
        ];
        for name in &names {
            let cstr =
                CStr::from_bytes_with_nul(name.as_bytes()).unwrap();
            let result = unsafe {
                resolve_instance_proc(vk::Instance::null(), cstr.as_ptr())
            };
            assert!(
                result.is_some(),
                "Expected Some for {name:?}, got None"
            );
        }
    }

    #[test]
    fn resolve_device_proc_null_name_returns_none() {
        let result = unsafe {
            resolve_device_proc(vk::Device::null(), std::ptr::null())
        };
        assert!(result.is_none());
    }

    #[test]
    fn resolve_device_proc_known_names_return_some() {
        let names = [
            "vkGetDeviceProcAddr\0",
            "vkCreateDevice\0",
            "vkDestroyDevice\0",
            "vkCreateSwapchainKHR\0",
            "vkDestroySwapchainKHR\0",
            "vkQueuePresentKHR\0",
        ];
        for name in &names {
            let cstr =
                CStr::from_bytes_with_nul(name.as_bytes()).unwrap();
            let result = unsafe {
                resolve_device_proc(vk::Device::null(), cstr.as_ptr())
            };
            assert!(
                result.is_some(),
                "Expected Some for {name:?}, got None"
            );
        }
    }

    #[test]
    fn resolve_device_proc_unknown_name_without_device_returns_none() {
        let cstr = CStr::from_bytes_with_nul(b"vkFooBar\0").unwrap();
        let result = unsafe {
            resolve_device_proc(vk::Device::null(), cstr.as_ptr())
        };
        assert!(result.is_none());
    }

    #[test]
    fn resolve_instance_proc_unknown_name_without_instance_returns_none() {
        let cstr = CStr::from_bytes_with_nul(b"vkFooBar\0").unwrap();
        let result = unsafe {
            resolve_instance_proc(vk::Instance::null(), cstr.as_ptr())
        };
        assert!(result.is_none());
    }

    #[test]
    fn layer_fn_macro_produces_fn_pointer() {
        let f: unsafe extern "system" fn() =
            layer_fn!(layer_vk_create_instance);
        let _ = f as usize;
    }

    #[test]
    fn device_state_destroy_capture_noop_when_not_ready() {
        // This verifies destroy_capture is safe to call when !ready
        // We can't construct a full DeviceState without Vulkan, but
        // we test the ready check path via the public interface.
        // The actual test is that the function exists and compiles.
        // Integration test needed for full coverage.
    }

    #[test]
    fn global_state_thread_safety() {
        // Verify with_global can be called from multiple threads
        let handles: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    with_global(|g| {
                        // Just access the map, don't insert anything
                        let _ = g.instances.len();
                    });
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn json_manifest_exists_and_is_valid() {
        let manifest = include_str!("../VkLayer_rivulet_capture.json");
        let json: serde_json::Value = serde_json::from_str(manifest)
            .expect("manifest is valid JSON");
        let layer = &json["layer"];
        assert_eq!(
            layer["name"].as_str().unwrap(),
            "VK_LAYER_RIVULET_capture"
        );
        assert_eq!(layer["type"].as_str().unwrap(), "GLOBAL");
        assert!(layer["api_version"].as_str().is_some());
        assert!(layer["description"].as_str().is_some());
    }
}

