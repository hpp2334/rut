// mandelbrot.js — must match workloads/mandelbrot.rut.
function mandelbrot(x0, y0, x1, y1, w, h, maxIter) {
  let checksum = 0;
  for (let y = 0; y < h; y++) {
    const ci = y0 + ((y1 - y0) * y) / h;
    for (let x = 0; x < w; x++) {
      const cr = x0 + ((x1 - x0) * x) / w;
      let zr = 0;
      let zi = 0;
      let i = 0;
      while (i < maxIter) {
        const zr2 = zr * zr;
        const zi2 = zi * zi;
        if (zr2 + zi2 > 4) break;
        zi = 2 * zr * zi + ci;
        zr = zr2 - zi2 + cr;
        i++;
      }
      checksum += i;
    }
  }
  return checksum;
}

const checksum = mandelbrot(-2, -1.2, 0.8, 1.2, 100, 75, 200);
console.log("CHECKSUM " + checksum);
