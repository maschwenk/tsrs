/* icount: exact user-space instruction count for a whole process, summed over all threads.
 *
 * Every application basic block adds its static instruction count to a per-thread counter
 * (inline: TLS read, load, add, store; no clean call, no shared cache line). Counters live in a
 * global list that is never freed, so threads that exited early and threads still alive at
 * process exit are both summed at the process exit event regardless of event ordering.
 *
 * Run with drrun -no_enable_reset (cache resets re-instrument and add noise). Do not raise -max_elide_jmp or
 * -max_elide_call above drrun's default of 0: elided direct branches leave the block and go uncounted.
 *
 * Client options: -out <path> (default: stderr), -per_thread (also print one line per thread).
 */
#include "dr_api.h"
#include "drmgr.h"
#include "drreg.h"
#include <string.h>

typedef struct per_thread {
    uint64 count; /* must stay at offset 0: the inlined update addresses it as [ptr] */
    struct per_thread *next;
    thread_id_t tid;
    uint seq;
} per_thread_t;

static int tls_idx = -1;
static void *list_lock;
static per_thread_t *all_threads;
static uint thread_seq;
static char out_path[1024];
static bool per_thread_lines;

static void
event_thread_init(void *drcontext)
{
    per_thread_t *pt = (per_thread_t *)dr_global_alloc(sizeof(*pt));
    memset(pt, 0, sizeof(*pt));
    pt->tid = dr_get_thread_id(drcontext);
    dr_mutex_lock(list_lock);
    pt->seq = thread_seq++;
    pt->next = all_threads;
    all_threads = pt;
    dr_mutex_unlock(list_lock);
    drmgr_set_tls_field(drcontext, tls_idx, pt);
}

static dr_emit_flags_t
event_bb_analysis(void *drcontext, void *tag, instrlist_t *bb, bool for_trace,
                  bool translating, void **user_data)
{
    uint n = 0;
    for (instr_t *in = instrlist_first_app(bb); in != NULL; in = instr_get_next_app(in))
        n++;
    *user_data = (void *)(ptr_uint_t)n;
    return DR_EMIT_DEFAULT;
}

static dr_emit_flags_t
event_app_instruction(void *drcontext, void *tag, instrlist_t *bb, instr_t *where,
                      bool for_trace, bool translating, void *user_data)
{
    uint n = (uint)(ptr_uint_t)user_data;
    drmgr_disable_auto_predication(drcontext, bb);
    if (!drmgr_is_first_instr(drcontext, where) || n == 0)
        return DR_EMIT_DEFAULT;
    reg_id_t ptr, val;
    if (drreg_reserve_register(drcontext, bb, where, NULL, &ptr) != DRREG_SUCCESS ||
        drreg_reserve_register(drcontext, bb, where, NULL, &val) != DRREG_SUCCESS)
        DR_ASSERT_MSG(false, "icount: drreg_reserve_register failed");
    drmgr_insert_read_tls_field(drcontext, tls_idx, bb, where, ptr);
    instrlist_meta_preinsert(
        bb, where,
        XINST_CREATE_load(drcontext, opnd_create_reg(val), OPND_CREATE_MEMPTR(ptr, 0)));
    /* AArch64 add-immediate takes 12 bits and leaves NZCV alone, so no flag spill. */
    while (n > 0) {
        uint chunk = n > 4095 ? 4095 : n;
        instrlist_meta_preinsert(
            bb, where,
            XINST_CREATE_add(drcontext, opnd_create_reg(val), OPND_CREATE_INT(chunk)));
        n -= chunk;
    }
    instrlist_meta_preinsert(
        bb, where,
        XINST_CREATE_store(drcontext, OPND_CREATE_MEMPTR(ptr, 0), opnd_create_reg(val)));
    if (drreg_unreserve_register(drcontext, bb, where, val) != DRREG_SUCCESS ||
        drreg_unreserve_register(drcontext, bb, where, ptr) != DRREG_SUCCESS)
        DR_ASSERT_MSG(false, "icount: drreg_unreserve_register failed");
    return DR_EMIT_DEFAULT;
}

static void
event_exit(void)
{
    uint64 total = 0;
    uint threads = 0;
    for (per_thread_t *pt = all_threads; pt != NULL; pt = pt->next) {
        total += pt->count;
        threads++;
    }
    file_t f = STDERR;
    if (out_path[0] != '\0') {
        f = dr_open_file(out_path, DR_FILE_WRITE_OVERWRITE);
        if (f == INVALID_FILE)
            f = STDERR;
    }
    dr_fprintf(f, "icount total=%llu threads=%u\n", total, threads);
    if (per_thread_lines) {
        for (per_thread_t *pt = all_threads; pt != NULL; pt = pt->next)
            dr_fprintf(f, "icount thread seq=%u tid=%d count=%llu\n", pt->seq, (int)pt->tid,
                       pt->count);
    }
    if (f != STDERR)
        dr_close_file(f);
    drmgr_unregister_tls_field(tls_idx);
    dr_mutex_destroy(list_lock);
    drreg_exit();
    drmgr_exit();
}

DR_EXPORT void
dr_client_main(client_id_t id, int argc, const char *argv[])
{
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-out") == 0 && i + 1 < argc) {
            dr_snprintf(out_path, sizeof(out_path), "%s", argv[++i]);
            out_path[sizeof(out_path) - 1] = '\0';
        } else if (strcmp(argv[i], "-per_thread") == 0) {
            per_thread_lines = true;
        } else {
            dr_fprintf(STDERR, "icount: unknown option %s\n", argv[i]);
            dr_abort();
        }
    }
    dr_set_client_name("icount (per-thread instruction count)", "");
    drreg_options_t ops = { sizeof(ops), 3 /* spill slots */, false };
    if (!drmgr_init() || drreg_init(&ops) != DRREG_SUCCESS)
        DR_ASSERT_MSG(false, "icount: init failed");
    list_lock = dr_mutex_create();
    tls_idx = drmgr_register_tls_field();
    DR_ASSERT(tls_idx != -1);
    drmgr_register_thread_init_event(event_thread_init);
    drmgr_register_exit_event(event_exit);
    drmgr_register_bb_instrumentation_event(event_bb_analysis, event_app_instruction, NULL);
}
