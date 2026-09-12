// nbody.js — must match workloads/nbody.rut (same Newton sqrt).
function fsqrt(x) {
  if (x <= 0) return 0;
  let r = x;
  for (let i = 0; i < 30; i++) r = 0.5 * (r + x / r);
  return r;
}

function energy(pos, vel, mass) {
  let e = 0;
  for (let i = 0; i < 5; i++) {
    e +=
      0.5 *
      mass[i] *
      (vel[i * 3] * vel[i * 3] +
        vel[i * 3 + 1] * vel[i * 3 + 1] +
        vel[i * 3 + 2] * vel[i * 3 + 2]);
    for (let j = i + 1; j < 5; j++) {
      const dx = pos[i * 3] - pos[j * 3];
      const dy = pos[i * 3 + 1] - pos[j * 3 + 1];
      const dz = pos[i * 3 + 2] - pos[j * 3 + 2];
      e -= (mass[i] * mass[j]) / fsqrt(dx * dx + dy * dy + dz * dz);
    }
  }
  return e;
}

function advance(pos, vel, mass, dt) {
  for (let i = 0; i < 5; i++) {
    for (let j = i + 1; j < 5; j++) {
      const dx = pos[i * 3] - pos[j * 3];
      const dy = pos[i * 3 + 1] - pos[j * 3 + 1];
      const dz = pos[i * 3 + 2] - pos[j * 3 + 2];
      const d2 = dx * dx + dy * dy + dz * dz;
      const mag = dt / (d2 * fsqrt(d2));
      vel[i * 3] -= dx * mass[j] * mag;
      vel[i * 3 + 1] -= dy * mass[j] * mag;
      vel[i * 3 + 2] -= dz * mass[j] * mag;
      vel[j * 3] += dx * mass[i] * mag;
      vel[j * 3 + 1] += dy * mass[i] * mag;
      vel[j * 3 + 2] += dz * mass[i] * mag;
    }
  }
  for (let i = 0; i < 5; i++) {
    pos[i * 3] += dt * vel[i * 3];
    pos[i * 3 + 1] += dt * vel[i * 3 + 1];
    pos[i * 3 + 2] += dt * vel[i * 3 + 2];
  }
}

const pi = 3.141592653589793;
const solarMass = 4 * pi * pi;
const daysPerYear = 365.24;

const mass = [
  solarMass,
  0.000954791938424327 * solarMass,
  0.000285885980666131 * solarMass,
  0.000043662440433516 * solarMass,
  0.000051513890204661 * solarMass,
];

const pos = [
  0, 0, 0,
  4.841431442464721, -1.160320044027428, -0.103622044471123,
  8.34336671824458, 4.124798564124305, -0.403523417114321,
  12.8943695621391, -15.1111514016986, -0.223307578892656,
  15.3796971148509, -25.9193146099879, 0.179258772950371,
];

const vel = [
  0, 0, 0,
  0.001660076642744037 * daysPerYear, 0.007699011184197404 * daysPerYear, -0.000069046001697206 * daysPerYear,
  -0.002767425107268624 * daysPerYear, 0.004998528012349172 * daysPerYear, 0.000023041729757376 * daysPerYear,
  0.002964601375647616 * daysPerYear, 0.00237847173959481 * daysPerYear, -0.000029658956854024 * daysPerYear,
  0.002680677724903893 * daysPerYear, 0.001628241700382423 * daysPerYear, -0.000095159225451972 * daysPerYear,
];

let mvx = 0, mvy = 0, mvz = 0;
for (let i = 0; i < 5; i++) {
  mvx += vel[i * 3] * mass[i];
  mvy += vel[i * 3 + 1] * mass[i];
  mvz += vel[i * 3 + 2] * mass[i];
}
vel[0] = -mvx / mass[0];
vel[1] = -mvy / mass[0];
vel[2] = -mvz / mass[0];

for (let s = 0; s < 1000; s++) advance(pos, vel, mass, 0.01);
console.log("CHECKSUM " + energy(pos, vel, mass));
