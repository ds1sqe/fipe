use std::error::Error;

use burn_tensor::{DType, Device, Tensor};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn metadata<const D: usize>(
    tensor: &Tensor<D>,
    device: &Device,
    shape: [usize; D],
) {
    assert_eq!(tensor.dtype(), DType::F32);
    assert_eq!(tensor.dims(), shape);
    assert_eq!(tensor.device(), *device);
}

fn read<const D: usize>(
    tensor: &Tensor<D>,
    device: &Device,
    shape: [usize; D],
) -> Result<Vec<f32>> {
    metadata(tensor, device, shape);
    device.sync()?;
    let data = tensor.clone().try_into_data()?;
    assert_eq!(data.dtype(), DType::F32);
    assert_eq!(data.shape().as_slice(), shape);
    Ok(data.try_into_vec::<f32>()?)
}

fn smoke(device: &Device, cpu: &Device) -> Result<()> {
    let xs = [1.0_f32, -2.0, 0.5, 4.0];
    let ys = [-1.0_f32, 2.0, 2.0, -0.5];
    let rows_a = [[1.0_f32, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let rows_b = [[7.0_f32, 8.0], [9.0, 10.0], [11.0, 12.0]];

    let mut add_reference = Vec::new();
    for (x, y) in xs.iter().zip(ys) {
        add_reference.push(f64::from(*x) + f64::from(y));
    }
    assert_eq!(add_reference, [0.0, 0.0, 2.5, 3.5]);
    let mut matmul_reference = Vec::new();
    for row in rows_a {
        for column in 0..2 {
            let mut sum = 0.0_f64;
            for (a, b_row) in row.iter().zip(rows_b) {
                sum += f64::from(*a) * f64::from(b_row[column]);
            }
            matmul_reference.push(sum);
        }
    }
    assert_eq!(matmul_reference, [58.0, 64.0, 139.0, 154.0]);

    let cpu_x = Tensor::<1>::from_data(xs, (cpu, DType::F32));
    let cpu_y = Tensor::<1>::from_data(ys, (cpu, DType::F32));
    let cpu_a = Tensor::<2>::from_data(rows_a, (cpu, DType::F32));
    let cpu_b = Tensor::<2>::from_data(rows_b, (cpu, DType::F32));
    metadata(&cpu_x, cpu, [4]);
    metadata(&cpu_y, cpu, [4]);
    metadata(&cpu_a, cpu, [2, 3]);
    metadata(&cpu_b, cpu, [3, 2]);

    let x = cpu_x.clone().to_device(device);
    let y = cpu_y.clone().to_device(device);
    let a = cpu_a.clone().to_device(device);
    let b = cpu_b.clone().to_device(device);
    metadata(&x, device, [4]);
    metadata(&y, device, [4]);
    metadata(&a, device, [2, 3]);
    metadata(&b, device, [3, 2]);
    let sum = x.clone() + y.clone();
    let product = a.clone().matmul(b.clone());
    metadata(&sum, device, [4]);
    metadata(&product, device, [2, 2]);

    let round_trip = read(&x, device, [4])?;
    let added = read(&sum, device, [4])?;
    let multiplied = read(&product, device, [2, 2])?;
    assert_eq!(round_trip, xs);
    assert_eq!(
        added.iter().copied().map(f64::from).collect::<Vec<_>>(),
        add_reference
    );
    assert_eq!(
        multiplied
            .iter()
            .copied()
            .map(f64::from)
            .collect::<Vec<_>>(),
        matmul_reference
    );
    println!("F32 round_trip [4] = {round_trip:?}");
    println!("F32 add [4] = {added:?}");
    println!("F32 matmul [2, 2] = {multiplied:?}");
    // Retain all CPU originals and transferred inputs until readbacks have completed.
    drop((cpu_x, cpu_y, cpu_a, cpu_b, x, y, a, b));
    Ok(())
}

#[cfg(feature = "gpu-vulkan")]
fn vulkan() -> Result<()> {
    use burn_tensor::wgpu::{wgpu, WgpuBackend};
    use burn_tensor::DeviceKind;

    let (gpu, setup) = Device::wgpu_options()
        .device_kind(DeviceKind::IntegratedGpu(0))
        .graphics_api(WgpuBackend::Vulkan)
        .init_with_setup()?;
    let info = setup.adapter.get_info();
    assert_eq!(setup.backend, wgpu::Backend::Vulkan);
    assert_eq!(info.backend, wgpu::Backend::Vulkan);
    assert_eq!(info.device_type, wgpu::DeviceType::IntegratedGpu);
    assert_eq!(info.vendor, 0x8086);
    assert_eq!(info.device, 0x7d45);
    println!(
        "adapter name={:?} vendor={:#06x} device={:#06x} type={:?} backend={:?}",
        info.name, info.vendor, info.device, info.device_type, info.backend
    );
    let cpu = Device::flex();
    smoke(&gpu, &cpu)?;
    println!("vulkan PASS");
    drop((gpu, setup));
    Ok(())
}

#[cfg(not(feature = "gpu-vulkan"))]
fn vulkan() -> Result<()> {
    Err("gpu-vulkan backend not compiled".into())
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [mode] if mode == "cpu" => {
            let cpu = Device::flex();
            smoke(&cpu, &cpu)?;
            println!("cpu PASS");
            Ok(())
        }
        [mode] if mode == "vulkan" => vulkan(),
        _ => Err("usage: fipe-tensor-smoke cpu|vulkan".into()),
    }
}
