//! Wrapper for handling samplers (`metal/Sampler.zig`).
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLDevice as _,
    MTLSamplerAddressMode,
    MTLSamplerDescriptor,
    MTLSamplerMinMagFilter,
    MTLSamplerState,
};

use super::{Device, Error};

/// Options for initializing a sampler.
#[derive(Clone)]
pub struct Options {
    /// MTLDevice
    pub device: Device,
    pub min_filter: MTLSamplerMinMagFilter,
    pub mag_filter: MTLSamplerMinMagFilter,
    pub s_address_mode: MTLSamplerAddressMode,
    pub t_address_mode: MTLSamplerAddressMode,
}

/// A sampler state.
pub struct Sampler {
    /// The underlying MTLSamplerState Object.
    sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
}

impl Sampler {
    /// The underlying MTLSamplerState.
    pub fn sampler(&self) -> &ProtocolObject<dyn MTLSamplerState> {
        &self.sampler
    }

    /// Initialize a sampler
    pub fn init(opts: &Options) -> Result<Sampler, Error> {
        // Create our descriptor and set the properties
        let desc = MTLSamplerDescriptor::new();
        desc.setMinFilter(opts.min_filter);
        desc.setMagFilter(opts.mag_filter);
        desc.setSAddressMode(opts.s_address_mode);
        desc.setTAddressMode(opts.t_address_mode);

        // Create the sampler state
        let sampler = opts
            .device
            .newSamplerStateWithDescriptor(&desc)
            .ok_or(Error::MetalFailed)?;

        Ok(Sampler { sampler })
    }
}
