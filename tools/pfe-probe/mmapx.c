// Throwaway: cost of mapping a cache image lazily vs reading it (persisted front-end design).
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec * 1e-9; }
static size_t PG;
typedef struct { volatile uint8_t *base; size_t npages; uint8_t *sel; int tid, nthreads, write; uint64_t sum; } job;
static void *touch(void *a) {
  job *j = a; uint64_t s = 0;
  for (size_t p = j->tid; p < j->npages; p += j->nthreads)
    if (j->sel[p]) { if (j->write) j->base[p * PG + 8] = 1; else s += j->base[p * PG + 8]; }
  j->sum = s; return 0;
}
static double run(volatile uint8_t *base, size_t npages, uint8_t *sel, int nthreads, int write) {
  pthread_t th[64]; job js[64]; double t0 = now();
  for (int i = 0; i < nthreads; i++) { js[i] = (job){base, npages, sel, i, nthreads, write, 0}; pthread_create(&th[i], 0, touch, &js[i]); }
  for (int i = 0; i < nthreads; i++) pthread_join(th[i], 0);
  return now() - t0;
}
int main(int argc, char **argv) {
  PG = (size_t)sysconf(_SC_PAGESIZE);
  const char *path = argv[1]; size_t mib = argc > 2 ? atol(argv[2]) : 512; double frac = argc > 3 ? atof(argv[3]) : 0.73;
  int nthreads = argc > 4 ? atoi(argv[4]) : 1;
  size_t size = mib << 20, npages = size / PG;
  struct stat st;
  if (stat(path, &st) != 0 || (size_t)st.st_size != size) {
    int fd = open(path, O_CREAT | O_TRUNC | O_WRONLY, 0644); char *buf = malloc(1 << 20);
    for (size_t i = 0; i < (1 << 20); i++) buf[i] = (char)(i * 2654435761u >> 13);
    for (size_t i = 0; i < mib; i++) write(fd, buf, 1 << 20);
    close(fd); free(buf);
  }
  uint8_t *sel = malloc(npages); srand(42);
  size_t nsel = 0; for (size_t p = 0; p < npages; p++) { sel[p] = (rand() / (double)RAND_MAX) < frac; nsel += sel[p]; }
  uint8_t *all = malloc(npages); memset(all, 1, npages);
  // warm the page cache
  { int fd = open(path, O_RDONLY); char *b = malloc(1 << 20); while (read(fd, b, 1 << 20) > 0) {} close(fd); free(b); }
  printf("page %zu, image %zu MiB, %zu pages, touching %zu (%.0f%%), threads %d\n", PG, mib, npages, nsel, 100.0 * nsel / npages, nthreads);
  for (int rep = 0; rep < 3; rep++) {
    // 1. read() the whole image into fresh anonymous memory
    double t0 = now(); int fd = open(path, O_RDONLY);
    uint8_t *buf = mmap(0, size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    size_t off = 0; while (off < size) { ssize_t r = read(fd, buf + off, size - off > (64 << 20) ? (64 << 20) : size - off); if (r <= 0) break; off += r; }
    close(fd); double t_read = now() - t0; munmap(buf, size);
    // 2. mmap and touch the selected pages (read)
    t0 = now(); fd = open(path, O_RDONLY);
    volatile uint8_t *m = mmap(0, size, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0); close(fd);
    double t_map = now() - t0;
    double t_touch = run(m, npages, sel, nthreads, 0);
    // 3. then write 23% of all pages (copy-on-write of touched pages)
    uint8_t *wsel = malloc(npages); for (size_t p = 0; p < npages; p++) wsel[p] = sel[p] && (p % 13) < 3;
    double t_cow = run(m, npages, wsel, nthreads, 1);
    munmap((void *)m, size); free(wsel);
    // 4. mmap and touch every page
    fd = open(path, O_RDONLY); m = mmap(0, size, PROT_READ, MAP_PRIVATE, fd, 0); close(fd);
    double t_all = run(m, npages, all, nthreads, 0); munmap((void *)m, size);
    // 5. zero-fill faults of fresh anonymous memory (what parsing into the arena pays)
    m = mmap(0, size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    double t_anon = run(m, npages, all, nthreads, 1); munmap((void *)m, size);
    printf("rep %d: read-all %.3fs | mmap %.4fs + touch-sel %.3fs (%.2f us/page) + cow %.3fs | touch-all %.3fs (%.2f us/page) | anon zero-fill %.3fs (%.2f us/page)\n",
           rep, t_read, t_map, t_touch, 1e6 * t_touch / nsel * nthreads, t_cow, t_all, 1e6 * t_all / npages * nthreads, t_anon, 1e6 * t_anon / npages * nthreads);
  }
  return 0;
}
