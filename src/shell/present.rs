//! Hands finished pixels to the window.
//!
//! On macOS the window shows one shared buffer (an IOSurface) that text9000
//! draws into directly: no second copy in text9000 and none in the window
//! server. Elsewhere softbuffer does the job, which on Windows and X11 also
//! keeps a single buffer.

use std::sync::Arc;

use winit::window::Window;

use super::raster::{IRect, Target};

pub struct Presenter {
    #[cfg(target_os = "macos")]
    inner: mac::Surface,
    #[cfg(not(target_os = "macos"))]
    inner: other::Surface,
}

impl Presenter {
    pub fn new(window: Arc<Window>) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        let inner = mac::Surface::new(&window)?;
        #[cfg(not(target_os = "macos"))]
        let inner = other::Surface::new(window)?;
        Ok(Self { inner })
    }

    /// Draws a frame of `width` x `height` pixels with `paint`, which gets the
    /// pixels and how many frames ago they were drawn (0: never) and returns
    /// what it changed. Nothing is sent to the screen when nothing changed.
    pub fn draw(
        &mut self,
        window: &Window,
        width: u32,
        height: u32,
        paint: impl FnOnce(&mut Target<'_>, u8) -> Vec<IRect>,
    ) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.inner.draw(window, width, height, paint)
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2_core_foundation::CGPoint;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use objc2_quartz_core::{CALayer, CATransaction};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use super::{IRect, Target};

    #[link(name = "IOSurface", kind = "framework")]
    unsafe extern "C" {
        fn IOSurfaceCreate(properties: *const c_void) -> *mut c_void;
        fn IOSurfaceLock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
        fn IOSurfaceUnlock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
        fn IOSurfaceGetBaseAddress(surface: *mut c_void) -> *mut c_void;
        fn IOSurfaceGetBytesPerRow(surface: *mut c_void) -> usize;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }

    pub struct Surface {
        root: Retained<CALayer>,
        layer: Retained<CALayer>,
        surface: *mut c_void,
        size: (u32, u32),
        drawn: bool,
    }

    impl Drop for Surface {
        fn drop(&mut self) {
            self.layer.removeFromSuperlayer();
            if !self.surface.is_null() {
                // SAFETY: we own the one reference IOSurfaceCreate gave us.
                unsafe { CFRelease(self.surface) };
            }
        }
    }

    impl Surface {
        pub fn new(window: &Window) -> Result<Self, String> {
            let handle = window.window_handle().map_err(|e| e.to_string())?;
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return Err("not an AppKit window".into());
            };
            // SAFETY: winit's handle points at the window's live NSView.
            let view: &AnyObject = unsafe { handle.ns_view.cast().as_ref() };
            let root: Option<Retained<CALayer>> = unsafe {
                let _: () = msg_send![view, setWantsLayer: Bool::YES];
                msg_send![view, layer]
            };
            let root = root.ok_or("the window has no layer")?;
            let layer = CALayer::new();
            layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
            // SAFETY: a constant from QuartzCore.
            layer.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityTopLeft });
            root.addSublayer(&layer);
            Ok(Self { root, layer, surface: std::ptr::null_mut(), size: (0, 0), drawn: false })
        }

        fn create(&mut self, width: u32, height: u32) -> Result<(), String> {
            if !self.surface.is_null() {
                // SAFETY: as in drop; the layer still holds its own reference.
                unsafe { CFRelease(self.surface) };
                self.surface = std::ptr::null_mut();
            }
            let keys = ["IOSurfaceWidth", "IOSurfaceHeight", "IOSurfaceBytesPerElement", "IOSurfacePixelFormat"]
                .map(NSString::from_str);
            let values = [
                NSNumber::new_u32(width),
                NSNumber::new_u32(height),
                NSNumber::new_u32(4),
                NSNumber::new_u32(u32::from_be_bytes(*b"BGRA")),
            ];
            let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
            let properties = NSDictionary::from_retained_objects(&keys, &values);
            // SAFETY: NSDictionary is toll-free bridged with CFDictionary.
            self.surface = unsafe { IOSurfaceCreate(Retained::as_ptr(&properties).cast()) };
            if self.surface.is_null() {
                return Err(format!("couldn't make a {width}x{height} screen buffer"));
            }
            self.size = (width, height);
            self.drawn = false;
            Ok(())
        }

        pub fn draw(
            &mut self,
            window: &Window,
            width: u32,
            height: u32,
            paint: impl FnOnce(&mut Target<'_>, u8) -> Vec<IRect>,
        ) -> Result<(), String> {
            let new = self.size != (width, height) || self.surface.is_null();
            if new {
                self.create(width, height)?;
            }
            // SAFETY: the surface is live; the lock makes the window server
            // finish reading it first, and the pixels are only touched while
            // it's held.
            let damage = unsafe {
                IOSurfaceLock(self.surface, 0, std::ptr::null_mut());
                let base = IOSurfaceGetBaseAddress(self.surface).cast::<u32>();
                let stride = IOSurfaceGetBytesPerRow(self.surface) / 4;
                let px = std::slice::from_raw_parts_mut(base, stride * height as usize);
                let mut target = Target { px, stride, width: width as usize, height: height as usize };
                let damage = paint(&mut target, if self.drawn { 1 } else { 0 });
                IOSurfaceUnlock(self.surface, 0, std::ptr::null_mut());
                damage
            };
            self.drawn = true;
            if damage.is_empty() && !new {
                return Ok(());
            }
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            if new {
                self.layer.setFrame(self.root.bounds());
                self.layer.setContentsScale(window.scale_factor());
                // SAFETY: an IOSurfaceRef is a valid layer contents object.
                let _: () = unsafe { msg_send![&*self.layer, setContents: self.surface.cast::<AnyObject>()] };
            } else {
                let _: () = unsafe { msg_send![&*self.layer, setContentsChanged] };
            }
            CATransaction::commit();
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use winit::window::Window;

    use super::{IRect, Target};

    pub struct Surface {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        size: (u32, u32),
    }

    impl Surface {
        pub fn new(window: Arc<Window>) -> Result<Self, String> {
            let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
            let surface = softbuffer::Surface::new(&context, window).map_err(|e| e.to_string())?;
            Ok(Self { surface, size: (0, 0) })
        }

        pub fn draw(
            &mut self,
            _window: &Window,
            width: u32,
            height: u32,
            paint: impl FnOnce(&mut Target<'_>, u8) -> Vec<IRect>,
        ) -> Result<(), String> {
            let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else { return Ok(()) };
            if self.size != (width, height) {
                self.surface.resize(w, h).map_err(|e| e.to_string())?;
                self.size = (width, height);
            }
            let mut buffer = self.surface.buffer_mut().map_err(|e| e.to_string())?;
            let age = buffer.age();
            let mut target = Target { px: &mut buffer, stride: width as usize, width: width as usize, height: height as usize };
            let damage = paint(&mut target, age);
            if damage.is_empty() && age != 0 {
                return Ok(());
            }
            let rects: Vec<softbuffer::Rect> = damage
                .iter()
                .filter_map(|r| {
                    Some(softbuffer::Rect {
                        x: r.x0 as u32,
                        y: r.y0 as u32,
                        width: NonZeroU32::new((r.x1 - r.x0) as u32)?,
                        height: NonZeroU32::new((r.y1 - r.y0) as u32)?,
                    })
                })
                .collect();
            if rects.is_empty() {
                buffer.present().map_err(|e| e.to_string())
            } else {
                buffer.present_with_damage(&rects).map_err(|e| e.to_string())
            }
        }
    }
}
