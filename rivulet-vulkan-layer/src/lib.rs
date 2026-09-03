#![allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code
)]

pub mod capture_channel;

use ash::vk;
use capture_channel::{next_sequence, FrameHeader};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use capture_channel::{DEFAULT_SHM_SIZE, SHM_NAME};
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Next-layer forwarding function pointer types
// ---------------------------------------------------------------------------

type FnGetInstanceProcAddr =
    unsafe extern "system" fn(vk::Instance, *const c_char) -> vk::PFN_vkVoidFunction;
type FnGetDeviceProcAddr =
    unsafe extern "system" fn(vk::Device, *const c_char) -> vk::PFN_vkVoidFunction;
type FnGetPhysicalDeviceProcAddr =
    unsafe extern "system" fn(vk::Instance, *const c_char) -> vk::PFN_vkVoidFunction;
type FnCreateInstance = unsafe extern "system" fn(
    *const vk::InstanceCreateInfo,
    *const vk::AllocationCallbacks,
    *mut vk::Instance,
) -> vk::Result;
type FnDestroyInstance = unsafe extern "system" fn(vk::Instance, *const vk::AllocationCallbacks);
type FnCreateDevice = unsafe extern "system" fn(
    vk::PhysicalDevice,
    *const vk::DeviceCreateInfo,
    *const vk::AllocationCallbacks,
    *mut vk::Device,
) -> vk::Result;
type FnDestroyDevice = unsafe extern "system" fn(vk::Device, *const vk::AllocationCallbacks);
type FnGetDeviceQueue = unsafe extern "system" fn(vk::Device, u32, u32, *mut vk::Queue);
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
// Loader-interface constants and structures (mirrors vulkan/vk_layer.h, which
// is not part of the core `ash` bindings).
// ---------------------------------------------------------------------------

const VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO: i32 = 47;
const VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO: i32 = 48;
const VK_LAYER_LINK_INFO: i32 = 0;
const LAYER_NEGOTIATE_INTERFACE_STRUCT: i32 = 1;

/// The layer implements the legacy loader interface (version 1), so the loader
/// resolves its entry points from the DLL's exported symbols.
const LAYER_LOADER_INTERFACE_VERSION: u32 = 1;

#[repr(C)]
struct VkLayerInstanceLink {
    p_next: *mut VkLayerInstanceLink,
    pfn_next_get_instance_proc_addr: FnGetInstanceProcAddr,
    pfn_next_get_physical_device_proc_addr: FnGetPhysicalDeviceProcAddr,
}

/// The `u` union of `VkLayerInstanceCreateInfo` is represented by its
/// `pLayerInfo` member: every other union arm is also pointer-sized, so a
/// single pointer field preserves the C layout exactly.
#[repr(C)]
struct VkLayerInstanceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    function: i32,
    p_layer_info: *mut VkLayerInstanceLink,
}

#[repr(C)]
struct VkLayerDeviceLink {
    p_next: *mut VkLayerDeviceLink,
    pfn_next_get_instance_proc_addr: FnGetInstanceProcAddr,
    pfn_next_get_device_proc_addr: FnGetDeviceProcAddr,
}

#[repr(C)]
struct VkLayerDeviceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    function: i32,
    p_layer_info: *mut VkLayerDeviceLink,
}

#[repr(C)]
pub struct VkNegotiateLayerInterface {
    s_type: i32,
    p_next: *mut c_void,
    loader_layer_interface_version: u32,
    pfn_get_instance_proc_addr: FnGetInstanceProcAddr,
    pfn_get_device_proc_addr: FnGetDeviceProcAddr,
    pfn_get_physical_device_proc_addr: FnGetPhysicalDeviceProcAddr,
}

/// Find the loader's `VkLayerInstanceCreateInfo` link entry in the pNext chain,
/// advance it so the next layer sees its own link, and return the next layer's
/// `vkGetInstanceProcAddr`.
///
/// The chain may contain several `VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO`
/// entries with different `function` values; only `VK_LAYER_LINK_INFO` carries
/// the forwarding pointer.
unsafe fn advance_instance_chain(mut p_next: *const c_void) -> Option<FnGetInstanceProcAddr> {
    while !p_next.is_null() {
        let s_type = *(p_next as *const i32);
        if s_type == VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO {
            let info = p_next as *mut VkLayerInstanceCreateInfo;
            if (*info).function == VK_LAYER_LINK_INFO {
                let link = (*info).p_layer_info;
                if link.is_null() {
                    return None;
                }
                let next_gipa = (*link).pfn_next_get_instance_proc_addr;
                (*info).p_layer_info = (*link).p_next;
                return Some(next_gipa);
            }
        }
        p_next = *((p_next as *const *const c_void).add(1));
    }
    None
}

/// Same as [`advance_instance_chain`], for the device chain.
unsafe fn advance_device_chain(
    mut p_next: *const c_void,
) -> Option<(FnGetInstanceProcAddr, FnGetDeviceProcAddr)> {
    while !p_next.is_null() {
        let s_type = *(p_next as *const i32);
        if s_type == VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO {
            let info = p_next as *mut VkLayerDeviceCreateInfo;
            if (*info).function == VK_LAYER_LINK_INFO {
                let link = (*info).p_layer_info;
                if link.is_null() {
                    return None;
                }
                let gipa = (*link).pfn_next_get_instance_proc_addr;
                let gdpa = (*link).pfn_next_get_device_proc_addr;
                (*info).p_layer_info = (*link).p_next;
                return Some((gipa, gdpa));
            }
        }
        p_next = *((p_next as *const *const c_void).add(1));
    }
    None
}

// ---------------------------------------------------------------------------
// Per-device capture state
// ---------------------------------------------------------------------------

struct DeviceState {
    device: ash::Device,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    graphics_queue_family: u32,
    fn_next_gdpa: FnGetDeviceProcAddr,
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
        self.cmd_pool = unsafe { self.device.create_command_pool(&pool_ci, None).unwrap() };

        let buf_ai = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        self.cmd_buf = unsafe { self.device.allocate_command_buffers(&buf_ai).unwrap()[0] };

        let bytes = ext.width as vk::DeviceSize * ext.height as vk::DeviceSize * 4;
        let buf_ci = vk::BufferCreateInfo::default()
            .size(bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        self.staging_buf = unsafe { self.device.create_buffer(&buf_ci, None).unwrap() };

        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(self.staging_buf) };
        let mem_props =
            unsafe { instance.get_physical_device_memory_properties(self.physical_device) };
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
        self.staging_mem = unsafe { self.device.allocate_memory(&alloc_ci, None).unwrap() };
        unsafe {
            self.device
                .bind_buffer_memory(self.staging_buf, self.staging_mem, 0)
                .unwrap();
        }
        self.staging_size = bytes;

        self.fence = unsafe {
            self.device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
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
            let frame =
                std::slice::from_raw_parts(ptr as *const u8, self.staging_size as usize).to_vec();
            self.device.unmap_memory(self.staging_mem);

            eprintln!("[Rivulet Layer] Captured frame: {} bytes", frame.len());
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

/// The layer is not shipped on other platforms (macOS etc.), so the shared
/// memory region is never created there.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn create_shm() -> Option<ShmHandle> {
    None
}

// ---------------------------------------------------------------------------
// Instance state
// ---------------------------------------------------------------------------

struct InstanceState {
    instance: ash::Instance,
    fn_next_gipa: FnGetInstanceProcAddr,
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

/// Layer negotiation entry point called by the Vulkan loader.
///
/// # Safety
/// `p_version_struct` must point to a valid `VkNegotiateLayerInterface` or be
/// null.
#[no_mangle]
pub unsafe extern "system" fn vkNegotiateLoaderLayerInterfaceVersion(
    p_version_struct: *mut VkNegotiateLayerInterface,
) -> vk::Result {
    if p_version_struct.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    let offered = (*p_version_struct).loader_layer_interface_version;
    if offered < 1 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    // We implement the legacy interface (version 1); negotiate down so the
    // loader resolves our entry points from the DLL's exported symbols.
    (*p_version_struct).loader_layer_interface_version =
        offered.min(LAYER_LOADER_INTERFACE_VERSION);
    eprintln!("[Rivulet Layer] negotiate offered={offered} -> 1");
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
        "vkGetInstanceProcAddr" => Some(layer_fn!(vkGetInstanceProcAddr)),
        "vkGetDeviceProcAddr" => Some(layer_fn!(vkGetDeviceProcAddr)),
        "vkCreateInstance" => Some(layer_fn!(layer_vk_create_instance)),
        "vkDestroyInstance" => Some(layer_fn!(layer_vk_destroy_instance)),
        "vkCreateDevice" => Some(layer_fn!(layer_vk_create_device)),
        "vkDestroyDevice" => Some(layer_fn!(layer_vk_destroy_device)),
        "vkCreateSwapchainKHR" => Some(layer_fn!(layer_vk_create_swapchain_khr)),
        "vkDestroySwapchainKHR" => Some(layer_fn!(layer_vk_destroy_swapchain_khr)),
        "vkQueuePresentKHR" => Some(layer_fn!(layer_vk_queue_present_khr)),
        _ => with_global(|g| {
            g.instances
                .get(&instance)
                .and_then(|i| (i.fn_next_gipa)(instance, name))
        }),
    }
}

unsafe fn resolve_device_proc(device: vk::Device, name: *const c_char) -> vk::PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    let s = CStr::from_ptr(name).to_string_lossy();
    match s.as_ref() {
        "vkGetDeviceProcAddr" => Some(layer_fn!(vkGetDeviceProcAddr)),
        "vkCreateDevice" => Some(layer_fn!(layer_vk_create_device)),
        "vkDestroyDevice" => Some(layer_fn!(layer_vk_destroy_device)),
        "vkCreateSwapchainKHR" => Some(layer_fn!(layer_vk_create_swapchain_khr)),
        "vkDestroySwapchainKHR" => Some(layer_fn!(layer_vk_destroy_swapchain_khr)),
        "vkQueuePresentKHR" => Some(layer_fn!(layer_vk_queue_present_khr)),
        _ => with_global(|g| {
            for inst in g.instances.values() {
                if let Some(dev) = inst.devices.get(&device) {
                    return (dev.fn_next_gdpa)(device, name);
                }
            }
            None
        }),
    }
}

/// Vulkan loader entry points. The loader looks these up under the generic
/// names (`vkGetInstanceProcAddr`, `vkGetDeviceProcAddr`) for interface
/// version 1 layers; the `VK_LAYER_RIVULET_capture_*` aliases are exported for
/// diagnostics and tooling.
///
/// # Safety
/// Called by the Vulkan loader; `p_name` must be a valid null-terminated
/// Vulkan command name or null.
#[no_mangle]
pub unsafe extern "system" fn vkGetInstanceProcAddr(
    instance: vk::Instance,
    p_name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    resolve_instance_proc(instance, p_name)
}

/// # Safety
/// Called by the Vulkan loader; `p_name` must be a valid null-terminated
/// Vulkan command name or null.
#[no_mangle]
pub unsafe extern "system" fn vkGetDeviceProcAddr(
    device: vk::Device,
    p_name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    resolve_device_proc(device, p_name)
}

/// # Safety
/// Called by the Vulkan loader; `p_name` must be a valid null-terminated
/// Vulkan command name or null.
#[no_mangle]
pub unsafe extern "system" fn VK_LAYER_RIVULET_capture_vkGetInstanceProcAddr(
    instance: vk::Instance,
    p_name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    resolve_instance_proc(instance, p_name)
}

/// # Safety
/// Called by the Vulkan loader; `p_name` must be a valid null-terminated
/// Vulkan command name or null.
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
    p_create_info: *const vk::InstanceCreateInfo,
    p_allocator: *const vk::AllocationCallbacks,
    p_instance: *mut vk::Instance,
) -> vk::Result {
    if p_create_info.is_null() || p_instance.is_null() {
        eprintln!("[Rivulet Layer] vkCreateInstance: null argument");
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let next_gipa = match advance_instance_chain((*p_create_info).p_next) {
        Some(f) => f,
        None => {
            eprintln!("[Rivulet Layer] vkCreateInstance: no instance chain link");
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        }
    };

    let create_name = CString::new("vkCreateInstance").unwrap();
    let next_create_instance: FnCreateInstance =
        match next_gipa(vk::Instance::null(), create_name.as_ptr()) {
            Some(f) => std::mem::transmute::<*const c_void, FnCreateInstance>(f as *const c_void),
            None => return vk::Result::ERROR_INITIALIZATION_FAILED,
        };

    // Forward creation down the chain, preserving the loader's pre-seeded
    // instance trampoline in `*p_instance` (the loader terminator reads it).
    let result = next_create_instance(p_create_info, p_allocator, p_instance);
    if result != vk::Result::SUCCESS {
        eprintln!("[Rivulet Layer] vkCreateInstance: next returned {result:?}");
        return result;
    }
    let instance = *p_instance;

    // Build an ash::Instance whose dispatch table resolves through the next
    // layer, so our physical-device queries and device creation reach the
    // driver below us (not ourselves).
    let ash_instance = ash::Instance::load_with(
        |name: &CStr| match next_gipa(instance, name.as_ptr()) {
            Some(f) => f as *const c_void,
            None => std::ptr::null(),
        },
        instance,
    );

    with_global(|g| {
        g.instances.insert(
            instance,
            InstanceState {
                instance: ash_instance,
                fn_next_gipa: next_gipa,
                devices: HashMap::new(),
            },
        );
    });

    eprintln!("[Rivulet Layer] vkCreateInstance → ok");
    vk::Result::SUCCESS
}

// ---------------------------------------------------------------------------
// vkDestroyInstance
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_destroy_instance(
    instance: vk::Instance,
    p_allocator: *const vk::AllocationCallbacks,
) {
    let mut removed = None;
    with_global(|g| {
        removed = g.instances.remove(&instance);
    });

    let mut next_gipa = None;
    if let Some(mut state) = removed {
        next_gipa = Some(state.fn_next_gipa);
        for (_, mut dev) in state.devices.drain() {
            dev.destroy_capture();
        }
        drop(state);
    }

    // Forward the destroy so the instance is actually torn down downstream.
    if let Some(next_gipa) = next_gipa {
        let name = CString::new("vkDestroyInstance").unwrap();
        if let Some(f) = next_gipa(instance, name.as_ptr()) {
            let destroy: FnDestroyInstance = std::mem::transmute(f);
            destroy(instance, p_allocator);
        }
    }
}

// ---------------------------------------------------------------------------
// vkCreateDevice
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_create_device(
    physical_device: vk::PhysicalDevice,
    p_create_info: *const vk::DeviceCreateInfo,
    p_allocator: *const vk::AllocationCallbacks,
    p_device: *mut vk::Device,
) -> vk::Result {
    if p_create_info.is_null() || p_device.is_null() {
        eprintln!("[Rivulet Layer] vkCreateDevice: null argument");
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    let ci = &*p_create_info;

    let (next_gipa, next_gdpa) = match advance_device_chain(ci.p_next) {
        Some(pair) => pair,
        None => {
            eprintln!("[Rivulet Layer] vkCreateDevice: no device chain link");
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        }
    };

    // Forward device creation down the chain.
    let create_name = CString::new("vkCreateDevice").unwrap();
    let next_create_device: FnCreateDevice =
        match next_gipa(vk::Instance::null(), create_name.as_ptr()) {
            Some(f) => std::mem::transmute::<*const c_void, FnCreateDevice>(f as *const c_void),
            None => return vk::Result::ERROR_INITIALIZATION_FAILED,
        };

    // Forward creation down the chain, preserving the loader's pre-seeded
    // device trampoline in `*p_device` (the loader terminator reads it).
    let result = next_create_device(physical_device, ci, p_allocator, p_device);
    if result != vk::Result::SUCCESS {
        eprintln!("[Rivulet Layer] vkCreateDevice: next returned {result:?}");
        return result;
    }
    let device = *p_device;

    // Locate our instance state (the capture model serves a single instance).
    let ash_instance =
        match with_global(|g| g.instances.iter().next().map(|(_, s)| s.instance.clone())) {
            Some(i) => i,
            None => return vk::Result::ERROR_INITIALIZATION_FAILED,
        };

    // Build an ash::Device resolving through the next layer for our capture
    // work (command pools, staging buffers, submissions).
    let ash_device = ash::Device::load(ash_instance.fp_v1_0(), device);
    let fn_destroy_device = ash_device.fp_v1_0().destroy_device;
    let fn_get_device_queue = ash_device.fp_v1_0().get_device_queue;

    // Load the extension functions we intercept/forward through the next layer.
    let load_ext = |name: &str| -> *const c_void {
        let cname = CString::new(name).unwrap();
        match next_gdpa(device, cname.as_ptr()) {
            Some(f) => f as *const c_void,
            None => std::ptr::null(),
        }
    };
    let fn_create_swapchain: FnCreateSwapchainKHR = std::mem::transmute::<
        *const c_void,
        FnCreateSwapchainKHR,
    >(load_ext("vkCreateSwapchainKHR"));
    let fn_destroy_swapchain: FnDestroySwapchainKHR = std::mem::transmute::<
        *const c_void,
        FnDestroySwapchainKHR,
    >(load_ext("vkDestroySwapchainKHR"));
    let fn_queue_present: FnQueuePresentKHR =
        std::mem::transmute::<*const c_void, FnQueuePresentKHR>(load_ext("vkQueuePresentKHR"));

    // Find the graphics queue family and queue.
    let queue_props = ash_instance.get_physical_device_queue_family_properties(physical_device);
    let graphics_qf = queue_props
        .iter()
        .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .unwrap_or(0) as u32;

    let mut graphics_queue = vk::Queue::null();
    fn_get_device_queue(device, graphics_qf, 0, &mut graphics_queue);

    let swapchain_loader = ash::khr::swapchain::Device::new(&ash_instance, &ash_device);

    let device_name = {
        let props = ash_instance.get_physical_device_properties(physical_device);
        CStr::from_ptr(props.device_name.as_ptr())
            .to_string_lossy()
            .into_owned()
    };

    let dev = DeviceState {
        device: ash_device,
        physical_device,
        graphics_queue,
        graphics_queue_family: graphics_qf,
        fn_next_gdpa: next_gdpa,
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

    eprintln!("[Rivulet Layer] vkCreateDevice → {device_name}");
    with_global(|g| {
        if let Some((_, state)) = g.instances.iter_mut().next() {
            state.devices.insert(device, dev);
        }
    });

    result
}

// ---------------------------------------------------------------------------
// vkDestroyDevice
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_destroy_device(
    device: vk::Device,
    p_allocator: *const vk::AllocationCallbacks,
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
        f(device, p_allocator);
    }
}

// ---------------------------------------------------------------------------
// vkCreateSwapchainKHR
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_create_swapchain_khr(
    device: vk::Device,
    p_create_info: *const vk::SwapchainCreateInfoKHR,
    p_allocator: *const vk::AllocationCallbacks,
    p_swapchain: *mut vk::SwapchainKHR,
) -> vk::Result {
    if p_create_info.is_null() || p_swapchain.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }

    let ci = &*p_create_info;

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
    let result = fn_create(device, ci, p_allocator, &mut swapchain);
    if result != vk::Result::SUCCESS {
        return result;
    }

    *p_swapchain = swapchain;

    with_global(|g| {
        if let Some(inst) = g.instances.get_mut(&inst_handle) {
            if let Some(dev) = inst.devices.get_mut(&device) {
                dev.swapchain = swapchain;
                let images = dev
                    .swapchain_loader
                    .get_swapchain_images(swapchain)
                    .unwrap_or_default();
                dev.init_capture(&inst.instance, &images, ci.image_format, ci.image_extent);
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
    p_allocator: *const vk::AllocationCallbacks,
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
        f(device, swapchain, p_allocator);
    }
}

// ---------------------------------------------------------------------------
// vkQueuePresentKHR — the capture point
// ---------------------------------------------------------------------------

unsafe extern "system" fn layer_vk_queue_present_khr(
    queue: vk::Queue,
    p_present_info: *const vk::PresentInfoKHR,
) -> vk::Result {
    if !p_present_info.is_null() {
        let pi = &*p_present_info;
        with_global(|g| {
            for inst in g.instances.values_mut() {
                for dev in inst.devices.values_mut() {
                    if dev.graphics_queue == queue && dev.ready {
                        let count = pi.swapchain_count as usize;
                        let indices = std::slice::from_raw_parts(pi.p_image_indices, count);
                        for &idx in indices {
                            if let Some(frame) = dev.capture(idx) {
                                write_frame_to_shm(&frame, dev.extent.width, dev.extent.height);
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
        Some(f) => f(queue, p_present_info),
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
        let result = unsafe { vkNegotiateLoaderLayerInterfaceVersion(std::ptr::null_mut()) };
        assert_eq!(result, vk::Result::ERROR_INITIALIZATION_FAILED);
    }
    #[test]
    fn negotiate_version_negotiates_down_to_legacy() {
        let mut s = VkNegotiateLayerInterface {
            s_type: LAYER_NEGOTIATE_INTERFACE_STRUCT,
            p_next: std::ptr::null_mut(),
            loader_layer_interface_version: 2,
            pfn_get_instance_proc_addr: vkGetInstanceProcAddr,
            pfn_get_device_proc_addr: vkGetDeviceProcAddr,
            pfn_get_physical_device_proc_addr: vkGetInstanceProcAddr,
        };
        let result = unsafe { vkNegotiateLoaderLayerInterfaceVersion(&mut s) };
        assert_eq!(result, vk::Result::SUCCESS);
        assert_eq!(
            s.loader_layer_interface_version,
            LAYER_LOADER_INTERFACE_VERSION
        );
    }

    #[test]
    fn negotiate_version_accepts_loader_version_one() {
        let mut s = VkNegotiateLayerInterface {
            s_type: LAYER_NEGOTIATE_INTERFACE_STRUCT,
            p_next: std::ptr::null_mut(),
            loader_layer_interface_version: 1,
            pfn_get_instance_proc_addr: vkGetInstanceProcAddr,
            pfn_get_device_proc_addr: vkGetDeviceProcAddr,
            pfn_get_physical_device_proc_addr: vkGetInstanceProcAddr,
        };
        let result = unsafe { vkNegotiateLoaderLayerInterfaceVersion(&mut s) };
        assert_eq!(result, vk::Result::SUCCESS);
        assert_eq!(s.loader_layer_interface_version, 1);
    }

    #[test]
    fn advance_instance_chain_skips_non_link_info_and_advances() {
        let next_gipa = vkGetInstanceProcAddr as FnGetInstanceProcAddr;
        let mut link = VkLayerInstanceLink {
            p_next: std::ptr::null_mut(),
            pfn_next_get_instance_proc_addr: next_gipa,
            pfn_next_get_physical_device_proc_addr: vkGetInstanceProcAddr
                as FnGetPhysicalDeviceProcAddr,
        };
        let link_info = VkLayerInstanceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO,
            p_next: std::ptr::null(),
            function: VK_LAYER_LINK_INFO,
            p_layer_info: &mut link,
        };
        let features = VkLayerInstanceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO,
            p_next: &link_info as *const _ as *const c_void,
            function: 3, // VK_LOADER_FEATURES (not a link)
            p_layer_info: std::ptr::null_mut(),
        };

        let found = unsafe { advance_instance_chain(&features as *const _ as *const c_void) };
        assert!(found.is_some());
        // The chain was advanced past `link`, which had no next link.
        assert!(link_info.p_layer_info.is_null());
    }

    #[test]
    fn advance_device_chain_returns_both_next_ptrs() {
        let gipa = vkGetInstanceProcAddr as FnGetInstanceProcAddr;
        let gdpa = vkGetDeviceProcAddr as FnGetDeviceProcAddr;
        let mut link = VkLayerDeviceLink {
            p_next: std::ptr::null_mut(),
            pfn_next_get_instance_proc_addr: gipa,
            pfn_next_get_device_proc_addr: gdpa,
        };
        let link_info = VkLayerDeviceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO,
            p_next: std::ptr::null(),
            function: VK_LAYER_LINK_INFO,
            p_layer_info: &mut link,
        };

        let found = unsafe { advance_device_chain(&link_info as *const _ as *const c_void) };
        assert!(found.is_some());
        assert!(link_info.p_layer_info.is_null());
    }

    #[test]
    fn resolve_instance_proc_null_name_returns_none() {
        let result = unsafe { resolve_instance_proc(vk::Instance::null(), std::ptr::null()) };
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
            let cstr = CStr::from_bytes_with_nul(name.as_bytes()).unwrap();
            let result = unsafe { resolve_instance_proc(vk::Instance::null(), cstr.as_ptr()) };
            assert!(result.is_some(), "Expected Some for {name:?}, got None");
        }
    }

    #[test]
    fn resolve_device_proc_null_name_returns_none() {
        let result = unsafe { resolve_device_proc(vk::Device::null(), std::ptr::null()) };
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
            let cstr = CStr::from_bytes_with_nul(name.as_bytes()).unwrap();
            let result = unsafe { resolve_device_proc(vk::Device::null(), cstr.as_ptr()) };
            assert!(result.is_some(), "Expected Some for {name:?}, got None");
        }
    }

    #[test]
    fn resolve_device_proc_unknown_name_without_device_returns_none() {
        let cstr = c"vkFooBar";
        let result = unsafe { resolve_device_proc(vk::Device::null(), cstr.as_ptr()) };
        assert!(result.is_none());
    }

    #[test]
    fn resolve_instance_proc_unknown_name_without_instance_returns_none() {
        let cstr = c"vkFooBar";
        let result = unsafe { resolve_instance_proc(vk::Instance::null(), cstr.as_ptr()) };
        assert!(result.is_none());
    }

    #[test]
    fn layer_fn_macro_produces_fn_pointer() {
        let f: unsafe extern "system" fn() = layer_fn!(layer_vk_create_instance);
        let _ = f as usize;
    }

    #[test]
    fn global_state_thread_safety() {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    with_global(|g| {
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
        let json: serde_json::Value =
            serde_json::from_str(manifest).expect("manifest is valid JSON");
        let layer = &json["layer"];
        assert_eq!(layer["name"].as_str().unwrap(), "VK_LAYER_RIVULET_capture");
        assert_eq!(layer["type"].as_str().unwrap(), "GLOBAL");
        assert!(layer["api_version"].as_str().is_some());
        assert!(layer["description"].as_str().is_some());
    }
}
