# boimp
boimp is the sound a mesh makes when its lods pop. it's also a library for octahedral imposters in bevy.

# versions
| boimp | bevy | note |
| --- | --- | --- |
| 0.1.0 | 0.14 | requires a slightly modified bevy 0.14.2 (see cargo.toml) |
| 0.2.0 | 0.15 | |

# bake
generate an imposter with an `ImposterBakeBundle`, specifying the image size, grid count, multisampling and grid mode (spherical / hemispherical / horizontal).

```rs
commands.spawn(ImposterBakeBundle {
    camera: ImposterBakeCamera {
        radius: 10.0, // how large an area to snapshot
        grid_size: 6, // 6x6 separate snapshots
        image_size: 512, // 512x512 texture
        grid_mode: GridMode::Spherical, // how the snapshots are arranged
        multisample: 8, // how many samples to average over (^2, 8 -> 64 samples)
        ..Default::default()
    };
    transform: Transform::from_translation(Vec3::ZERO),
})
```

for anything to be produced, the materials used in the area must implement `ImposterBakeMaterial`. This is automatically implemented for `StandardMaterial`s, other implementations can be registered by adding an `ImposterBakeMaterialPlugin::<M>`. the frag shader is quite simple, see [the standard material version](src/shaders/standard_material_imposter_baker.wgsl).

# render
render the imposter with a `MaterialMeshBundle`:

```rs
commands.spawn(MaterialMeshBundle::<Imposter> {
    mesh: meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5)).mesh()),
    material: asset_server.load_with_settings::<_, ImposterLoaderSettings>(source, move |s| {
        s.multisample = multisample;
    }),
    ..default()
});
```

Use a `Rectangle` or `Plane3d::new(Vec3::Z, Vec2::splat(0.5))` mesh. 

# examples:
## `dynamic` 
runs baking every frame (once 'I' is pressed, and until 'O' is pressed), and spawns a large number of imposters based on the bake results.

args:
- `--grid <n>` : number of separate snapshots (^2) (default 15)
- `--image <n>` : total size of texture image (^2) (default 1024)
- `--mode [s]pherical | [h]emispherical | [H]orizontal` : how the snapshots are arranged (default hemispherical)
- `--count <n>` : number of imposters to spawn (default 1000)
- `--source <path>` : gltf to load (default FlightHelmet)
- `--multisample-source <n>` : how many samples to average over when baking (^2) (default 1)
- `--multisample-target` : average samples over nearby material pixels when rendering imposters (default false)


## `save_asset`
loads a gltf, bakes and saves an imposter, with baking params from the commandine.

args:
- `--grid <n>` : number of separate snapshots (^2) (default 8)
- `--image <n>` : total size of texture image (^2) (default 512)
- `--mode [s]pherical | [h]emispherical | [H]orizontal` : how the snapshots are arranged (default hemispherical)
- `--source <path>` : gltf to load (default FlightHelmet)
- `--multisample <n>` : how many samples to average over when baking (^2) (default 8)
- `--output <path>` : where to output to (default "assets/boimps/output.boimp")


## `load_asset`
loads a previously baked imposter, with rendering params from the command line.

args:
- `--source <path>` : gltf to load (default "assets/boimps/output.boimp")
- `--multisample` : average samples over nearby material pixels when rendering imposters (default false)

# known issues

non-opaque materials aren't well supported. a single alpha-blend texture will work fine but multiple overlapping texture layers will take only the alpha of the front-most layer.

# todo
- [ ] integrate with visibility ranges
- [ ] improve asset format
- [x] store/adjust for depths
- [ ] maybe make the storage more configurable maybe - currently 5bit/channel color and alpha, 4bit metallic and roughness, 4bit flags (only unlit flag currently passed), 24bit normal, 8bit depth
- [ ] maybe add "image" mode that records the actual view rather than the material properties
- [x] update to 0.15 and upstream
- [ ] fix alpha issues
- [ ] use vertex instancing to avoid needing a mesh

## License

boimp is free and open source. All code in this repository is dual-licensed under either:

- MIT License ([LICENSE-MIT](/LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](/LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.
