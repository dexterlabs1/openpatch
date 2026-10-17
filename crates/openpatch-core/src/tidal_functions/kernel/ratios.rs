// Preserve the pinned source arithmetic/decimal constants and update order.
#![allow(clippy::excessive_precision)]

// Copyright 2017 Emilie Gillet.
//
// Author: Emilie Gillet (emilie.o.gillet@gmail.com)
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//
// See http://creativecommons.org/licenses/MIT/ for more information.
//
// Exact ratio initializers from pinned poly_slope_generator.cc.
// SHA-256: 3765745bd26b80469cd9ee7fca93c1a339b8e719ec626a9f005a439d1d07610a
use super::Ratio;

pub(super) const AUDIO_RATIO_TABLE: [[Ratio; 4]; 21] = [
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.5, q: 2 },
        Ratio { ratio: 0.25, q: 4 },
        Ratio { ratio: 0.125, q: 8 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.5, q: 2 },
        Ratio {
            ratio: 0.33333333,
            q: 3,
        },
        Ratio { ratio: 0.2, q: 5 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.5, q: 2 },
        Ratio {
            ratio: 0.33333333,
            q: 3,
        },
        Ratio { ratio: 0.25, q: 4 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio {
            ratio: 0.44444444,
            q: 9,
        },
        Ratio {
            ratio: 0.296296297,
            q: 27,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
        Ratio {
            ratio: 0.33333333,
            q: 3,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.75, q: 4 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.790123456,
            q: 81,
        },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.790123456,
            q: 81,
        },
        Ratio { ratio: 0.75, q: 4 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.88888888,
            q: 9,
        },
        Ratio {
            ratio: 0.790123456,
            q: 81,
        },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.99090909091,
            q: 109,
        },
        Ratio {
            ratio: 0.987341772,
            q: 79,
        },
        Ratio {
            ratio: 0.9811320755,
            q: 53,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 1.009174312,
            q: 109,
        },
        Ratio {
            ratio: 1.01265823,
            q: 79,
        },
        Ratio {
            ratio: 1.0188679245,
            q: 53,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.125, q: 8 },
        Ratio {
            ratio: 1.265625,
            q: 64,
        },
        Ratio { ratio: 1.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 1.265625,
            q: 64,
        },
        Ratio {
            ratio: 1.3333333,
            q: 3,
        },
        Ratio { ratio: 1.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 1.265625,
            q: 64,
        },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 1.33333333,
            q: 3,
        },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 3.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.25, q: 4 },
        Ratio { ratio: 3.375, q: 8 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 3.0, q: 1 },
        Ratio { ratio: 4.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 3.0, q: 1 },
        Ratio { ratio: 5.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 4.0, q: 1 },
        Ratio { ratio: 8.0, q: 1 },
    ],
];

pub(super) const CONTROL_RATIO_TABLE: [[Ratio; 4]; 21] = [
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.5, q: 2 },
        Ratio { ratio: 0.25, q: 4 },
        Ratio { ratio: 0.125, q: 8 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.5, q: 2 },
        Ratio {
            ratio: 0.33333333,
            q: 3,
        },
        Ratio { ratio: 0.2, q: 5 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.5, q: 2 },
        Ratio {
            ratio: 0.33333333,
            q: 3,
        },
        Ratio { ratio: 0.25, q: 4 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
        Ratio { ratio: 0.25, q: 4 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
        Ratio {
            ratio: 0.33333333,
            q: 3,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.75, q: 4 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.8, q: 5 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
        Ratio { ratio: 0.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.8, q: 5 },
        Ratio { ratio: 0.75, q: 3 },
        Ratio { ratio: 0.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 0.8, q: 5 },
        Ratio { ratio: 0.75, q: 4 },
        Ratio {
            ratio: 0.66666666,
            q: 3,
        },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 0.909090909091,
            q: 11,
        },
        Ratio {
            ratio: 0.857142857143,
            q: 7,
        },
        Ratio { ratio: 0.8, q: 5 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 1.09090909091,
            q: 11,
        },
        Ratio {
            ratio: 1.142857143,
            q: 7,
        },
        Ratio { ratio: 1.2, q: 5 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.25, q: 4 },
        Ratio {
            ratio: 1.33333333,
            q: 3,
        },
        Ratio { ratio: 1.5, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.25, q: 4 },
        Ratio {
            ratio: 1.33333333,
            q: 3,
        },
        Ratio { ratio: 2.0, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.25, q: 4 },
        Ratio { ratio: 1.5, q: 3 },
        Ratio { ratio: 2.0, q: 2 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio {
            ratio: 1.33333333,
            q: 3,
        },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 3.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 1.5, q: 2 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 4.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 3.0, q: 1 },
        Ratio { ratio: 4.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 3.0, q: 1 },
        Ratio { ratio: 5.0, q: 1 },
    ],
    [
        Ratio { ratio: 1.0, q: 1 },
        Ratio { ratio: 2.0, q: 1 },
        Ratio { ratio: 4.0, q: 1 },
        Ratio { ratio: 8.0, q: 1 },
    ],
];
