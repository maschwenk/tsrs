/* bbcount: per-basic-block execution counts of the main module (for an instruction profile).
 * Each distinct (block start, length) gets one record; the block's entry adds 1 to the record's counter
 * (non-atomic: use single-threaded runs for exact counts). At exit, writes "offset ninstr count" lines for
 * blocks of the main module and one "other <instructions>" line for everything else.
 * Run with drrun -no_enable_reset. Option: -out <path>. */
#include "dr_api.h"
#include "drmgr.h"
#include "drreg.h"
#include "hashtable.h"
#include <string.h>

typedef struct rec {
    uint64 count;
    app_pc start;
    uint ninstr;
    bool main;
} rec_t;

static hashtable_t table;
static void *lock;
static char out_path[1024];
static app_pc main_base, main_end;

static void free_rec(void *p) { (void)p; }

static rec_t *
get_rec(app_pc start, uint n)
{
    ptr_uint_t key = (ptr_uint_t)start ^ ((ptr_uint_t)n << 52);
    dr_mutex_lock(lock);
    rec_t *r = (rec_t *)hashtable_lookup(&table, (void *)key);
    if (r == NULL) {
        r = (rec_t *)dr_global_alloc(sizeof(*r));
        memset(r, 0, sizeof(*r));
        r->start = start;
        r->ninstr = n;
        r->main = start >= main_base && start < main_end;
        hashtable_add(&table, (void *)key, r);
    }
    dr_mutex_unlock(lock);
    return r;
}

static dr_emit_flags_t
event_bb_analysis(void *drcontext, void *tag, instrlist_t *bb, bool for_trace, bool translating, void **user_data)
{
    uint n = 0;
    app_pc start = NULL;
    for (instr_t *in = instrlist_first_app(bb); in != NULL; in = instr_get_next_app(in)) {
        if (start == NULL)
            start = instr_get_app_pc(in);
        n++;
    }
    *user_data = n == 0 ? NULL : get_rec(start, n);
    return DR_EMIT_DEFAULT;
}

static dr_emit_flags_t
event_app_instruction(void *drcontext, void *tag, instrlist_t *bb, instr_t *where, bool for_trace, bool translating,
                      void *user_data)
{
    rec_t *r = (rec_t *)user_data;
    drmgr_disable_auto_predication(drcontext, bb);
    if (!drmgr_is_first_instr(drcontext, where) || r == NULL)
        return DR_EMIT_DEFAULT;
    reg_id_t ptr, val;
    if (drreg_reserve_register(drcontext, bb, where, NULL, &ptr) != DRREG_SUCCESS ||
        drreg_reserve_register(drcontext, bb, where, NULL, &val) != DRREG_SUCCESS)
        DR_ASSERT_MSG(false, "bbcount: drreg_reserve_register failed");
    instrlist_insert_mov_immed_ptrsz(drcontext, (ptr_int_t)&r->count, opnd_create_reg(ptr), bb, where, NULL, NULL);
    instrlist_meta_preinsert(bb, where, XINST_CREATE_load(drcontext, opnd_create_reg(val), OPND_CREATE_MEMPTR(ptr, 0)));
    instrlist_meta_preinsert(bb, where, XINST_CREATE_add(drcontext, opnd_create_reg(val), OPND_CREATE_INT(1)));
    instrlist_meta_preinsert(bb, where, XINST_CREATE_store(drcontext, OPND_CREATE_MEMPTR(ptr, 0), opnd_create_reg(val)));
    if (drreg_unreserve_register(drcontext, bb, where, val) != DRREG_SUCCESS ||
        drreg_unreserve_register(drcontext, bb, where, ptr) != DRREG_SUCCESS)
        DR_ASSERT_MSG(false, "bbcount: drreg_unreserve_register failed");
    return DR_EMIT_DEFAULT;
}

static void
event_exit(void)
{
    file_t f = STDERR;
    if (out_path[0] != '\0') {
        f = dr_open_file(out_path, DR_FILE_WRITE_OVERWRITE);
        if (f == INVALID_FILE)
            f = STDERR;
    }
    uint64 other = 0, mainc = 0;
    for (uint i = 0; i < HASHTABLE_SIZE(table.table_bits); i++) {
        for (hash_entry_t *e = table.table[i]; e != NULL; e = e->next) {
            rec_t *r = (rec_t *)e->payload;
            if (r->count == 0)
                continue;
            if (r->main) {
                dr_fprintf(f, "%llx %u %llu\n", (uint64)(r->start - main_base), r->ninstr, r->count);
                mainc += r->count * r->ninstr;
            } else {
                other += r->count * r->ninstr;
            }
        }
    }
    dr_fprintf(f, "other %llu\nmain %llu\n", other, mainc);
    if (f != STDERR)
        dr_close_file(f);
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
        }
    }
    dr_set_client_name("bbcount", "");
    drreg_options_t ops = { sizeof(ops), 3, false };
    if (!drmgr_init() || drreg_init(&ops) != DRREG_SUCCESS)
        DR_ASSERT_MSG(false, "bbcount: init failed");
    module_data_t *m = dr_get_main_module();
    main_base = m->start;
    main_end = m->end;
    dr_free_module_data(m);
    lock = dr_mutex_create();
    hashtable_init_ex(&table, 20, HASH_INTPTR, false, false, free_rec, NULL, NULL);
    drmgr_register_exit_event(event_exit);
    drmgr_register_bb_instrumentation_event(event_bb_analysis, event_app_instruction, NULL);
}
