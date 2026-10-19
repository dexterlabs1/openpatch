// Mechanically extracted source settings.cc scalars; no DSP.
pub(super) fn preset_scales() -> [super::kernel::quantizer::Scale; 6] {
    use super::kernel::quantizer::{Degree, Scale};
    [
        Scale {
            base_interval: f32::from_bits(1065353216),
            num_degrees: 12,
            degree: [
                Degree {
                    voltage: f32::from_bits(0),
                    weight: 255,
                },
                Degree {
                    voltage: f32::from_bits(1034590513),
                    weight: 16,
                },
                Degree {
                    voltage: f32::from_bits(1042985832),
                    weight: 96,
                },
                Degree {
                    voltage: f32::from_bits(1048576000),
                    weight: 24,
                },
                Degree {
                    voltage: f32::from_bits(1051371084),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1054169524),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1056964608),
                    weight: 8,
                },
                Degree {
                    voltage: f32::from_bits(1058362150),
                    weight: 192,
                },
                Degree {
                    voltage: f32::from_bits(1059761370),
                    weight: 16,
                },
                Degree {
                    voltage: f32::from_bits(1061158912),
                    weight: 96,
                },
                Degree {
                    voltage: f32::from_bits(1062556454),
                    weight: 24,
                },
                Degree {
                    voltage: f32::from_bits(1063955674),
                    weight: 128,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
            ],
        },
        Scale {
            base_interval: f32::from_bits(1065353216),
            num_degrees: 12,
            degree: [
                Degree {
                    voltage: f32::from_bits(0),
                    weight: 255,
                },
                Degree {
                    voltage: f32::from_bits(1034590513),
                    weight: 16,
                },
                Degree {
                    voltage: f32::from_bits(1042985832),
                    weight: 96,
                },
                Degree {
                    voltage: f32::from_bits(1048576000),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1051371084),
                    weight: 8,
                },
                Degree {
                    voltage: f32::from_bits(1054169524),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1056964608),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1058362150),
                    weight: 192,
                },
                Degree {
                    voltage: f32::from_bits(1059761370),
                    weight: 96,
                },
                Degree {
                    voltage: f32::from_bits(1061158912),
                    weight: 16,
                },
                Degree {
                    voltage: f32::from_bits(1062556454),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1063955674),
                    weight: 16,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
            ],
        },
        Scale {
            base_interval: f32::from_bits(1065353216),
            num_degrees: 12,
            degree: [
                Degree {
                    voltage: f32::from_bits(0),
                    weight: 255,
                },
                Degree {
                    voltage: f32::from_bits(1034590513),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1042985832),
                    weight: 96,
                },
                Degree {
                    voltage: f32::from_bits(1048576000),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1051371084),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1054169524),
                    weight: 140,
                },
                Degree {
                    voltage: f32::from_bits(1056964608),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1058362150),
                    weight: 192,
                },
                Degree {
                    voltage: f32::from_bits(1059761370),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1061158912),
                    weight: 96,
                },
                Degree {
                    voltage: f32::from_bits(1062556454),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1063955674),
                    weight: 4,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
            ],
        },
        Scale {
            base_interval: f32::from_bits(1065353216),
            num_degrees: 7,
            degree: [
                Degree {
                    voltage: f32::from_bits(0),
                    weight: 255,
                },
                Degree {
                    voltage: f32::from_bits(1040355164),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1048995430),
                    weight: 32,
                },
                Degree {
                    voltage: f32::from_bits(1055622431),
                    weight: 8,
                },
                Degree {
                    voltage: f32::from_bits(1058446036),
                    weight: 192,
                },
                Degree {
                    voltage: f32::from_bits(1060432459),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1063368471),
                    weight: 16,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
            ],
        },
        Scale {
            base_interval: f32::from_bits(1065353216),
            num_degrees: 12,
            degree: [
                Degree {
                    voltage: f32::from_bits(0),
                    weight: 255,
                },
                Degree {
                    voltage: f32::from_bits(1033503349),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1043200580),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1049012208),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1050988564),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1054112481),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1056689462),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1058390671),
                    weight: 192,
                },
                Degree {
                    voltage: f32::from_bits(1059650640),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1061241120),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1062801401),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1063791257),
                    weight: 64,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
            ],
        },
        Scale {
            base_interval: f32::from_bits(1065353216),
            num_degrees: 12,
            degree: [
                Degree {
                    voltage: f32::from_bits(0),
                    weight: 255,
                },
                Degree {
                    voltage: f32::from_bits(1033503349),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1043200580),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1049012208),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1050988564),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1054112481),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1056689462),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1058390671),
                    weight: 192,
                },
                Degree {
                    voltage: f32::from_bits(1059650640),
                    weight: 4,
                },
                Degree {
                    voltage: f32::from_bits(1061241120),
                    weight: 64,
                },
                Degree {
                    voltage: f32::from_bits(1062801401),
                    weight: 128,
                },
                Degree {
                    voltage: f32::from_bits(1063791257),
                    weight: 4,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
                Degree {
                    voltage: 0.0,
                    weight: 0,
                },
            ],
        },
    ]
}
