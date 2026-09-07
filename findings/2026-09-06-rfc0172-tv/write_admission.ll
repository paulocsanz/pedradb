; ModuleID = 'write_admission_kernel.69d80c15212624c1-cgu.0'
source_filename = "write_admission_kernel.69d80c15212624c1-cgu.0"
target datalayout = "e-m:o-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-n32:64-S128-Fn32"
target triple = "arm64-apple-macosx11.0.0"

; write_admission_kernel::write_admit
; Function Attrs: uwtable
define i8 @_RNvCs95p5YarzxTh_22write_admission_kernel11write_admit(i64 %mem_bytes, i1 zeroext %mem_armed, i64 %mem_limit, i64 %l0, i1 zeroext %l0_armed, i64 %l0_limit) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  br i1 %mem_armed, label %bb1, label %bb3

bb3:                                              ; preds = %bb1, %start
  br i1 %l0_armed, label %bb4, label %bb6

bb1:                                              ; preds = %start
  %_7 = icmp uge i64 %mem_bytes, %mem_limit
  br i1 %_7, label %bb2, label %bb3

bb2:                                              ; preds = %bb1
  store i8 1, ptr %_0, align 1
  br label %bb7

bb6:                                              ; preds = %bb4, %bb3
  store i8 0, ptr %_0, align 1
  br label %bb7

bb4:                                              ; preds = %bb3
  %_8 = icmp uge i64 %l0, %l0_limit
  br i1 %_8, label %bb5, label %bb6

bb5:                                              ; preds = %bb4
  store i8 2, ptr %_0, align 1
  br label %bb7

bb7:                                              ; preds = %bb2, %bb5, %bb6
  %0 = load i8, ptr %_0, align 1
  ret i8 %0
}

; write_admission_kernel::seq_exhausted
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel13seq_exhausted(i64 %seq, i64 %max) unnamed_addr #0 {
start:
  %_0 = icmp ugt i64 %seq, %max
  ret i1 %_0
}

; write_admission_kernel::batch_is_empty
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel14batch_is_empty(i64 %n) unnamed_addr #0 {
start:
  %_0 = icmp eq i64 %n, 0
  ret i1 %_0
}

; write_admission_kernel::seq_after_feed
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel14seq_after_feed(i64 %seq, i64 %feed_max) unnamed_addr #0 {
start:
  %_0 = icmp ugt i64 %seq, %feed_max
  ret i1 %_0
}

; write_admission_kernel::dir_sync_required
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel17dir_sync_required(i1 zeroext %sync) unnamed_addr #0 {
start:
  ret i1 %sync
}

; write_admission_kernel::wal_sync_required
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel17wal_sync_required(i1 zeroext %client_set, i1 zeroext %client_sync, i1 zeroext %db_sync) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  br i1 %client_set, label %bb1, label %bb2

bb2:                                              ; preds = %start
  %0 = zext i1 %db_sync to i8
  store i8 %0, ptr %_0, align 1
  br label %bb3

bb1:                                              ; preds = %start
  %1 = zext i1 %client_sync to i8
  store i8 %1, ptr %_0, align 1
  br label %bb3

bb3:                                              ; preds = %bb1, %bb2
  %2 = load i8, ptr %_0, align 1
  %3 = trunc nuw i8 %2 to i1
  ret i1 %3
}

; write_admission_kernel::write_admit_as_is
; Function Attrs: uwtable
define i8 @_RNvCs95p5YarzxTh_22write_admission_kernel17write_admit_as_is(i64 %mem_bytes, i1 zeroext %mem_armed, i64 %mem_limit, i64 %l0, i1 zeroext %l0_armed, i64 %l0_limit) unnamed_addr #0 {
start:
  ret i8 0
}

; write_admission_kernel::fence_on_sync_fail
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel18fence_on_sync_fail(i1 zeroext %sync_required, i1 zeroext %sync_failed) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  br i1 %sync_required, label %bb1, label %bb2

bb2:                                              ; preds = %start
  store i8 0, ptr %_0, align 1
  br label %bb3

bb1:                                              ; preds = %start
  %0 = zext i1 %sync_failed to i8
  store i8 %0, ptr %_0, align 1
  br label %bb3

bb3:                                              ; preds = %bb1, %bb2
  %1 = load i8, ptr %_0, align 1
  %2 = trunc nuw i8 %1 to i1
  ret i1 %2
}

; write_admission_kernel::seq_exhausted_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel19seq_exhausted_as_is(i64 %_seq, i64 %_max) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::torn_tail_needs_cut
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel19torn_tail_needs_cut(i64 %len, i64 %last_good) unnamed_addr #0 {
start:
  %_0 = icmp ugt i64 %len, %last_good
  ret i1 %_0
}

; write_admission_kernel::batch_is_empty_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel20batch_is_empty_as_is(i64 %_n) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::seq_after_feed_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel20seq_after_feed_as_is(i64 %_seq, i64 %_feed_max) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::write_admission_idle
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel20write_admission_idle(i1 zeroext %mem_stall, i1 zeroext %pressure_l0, i1 zeroext %stall_l0) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  br i1 %mem_stall, label %bb3, label %bb1

bb1:                                              ; preds = %start
  br i1 %pressure_l0, label %bb3, label %bb2

bb3:                                              ; preds = %bb1, %start
  store i8 0, ptr %_0, align 1
  br label %bb4

bb2:                                              ; preds = %bb1
  %0 = xor i1 %stall_l0, true
  %1 = zext i1 %0 to i8
  store i8 %1, ptr %_0, align 1
  br label %bb4

bb4:                                              ; preds = %bb3, %bb2
  %2 = load i8, ptr %_0, align 1
  %3 = trunc nuw i8 %2 to i1
  ret i1 %3
}

; write_admission_kernel::torn_head_is_empty_log
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel22torn_head_is_empty_log(i64 %len, i64 %tiny_max) unnamed_addr #0 {
start:
  %_0 = icmp ult i64 %len, %tiny_max
  ret i1 %_0
}

; write_admission_kernel::dir_sync_required_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel23dir_sync_required_as_is(i1 zeroext %_sync) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::wal_sync_required_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel23wal_sync_required_as_is(i1 zeroext %_client_set, i1 zeroext %_client_sync, i1 zeroext %_db_sync) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::fence_on_sync_fail_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel24fence_on_sync_fail_as_is(i1 zeroext %_sync_required, i1 zeroext %_sync_failed) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::pit_resync_needs_rewrite
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel24pit_resync_needs_rewrite(i1 zeroext %is_resync) unnamed_addr #0 {
start:
  ret i1 %is_resync
}

; write_admission_kernel::torn_tail_needs_cut_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel25torn_tail_needs_cut_as_is(i64 %_len, i64 %_last_good) unnamed_addr #0 {
start:
  ret i1 false
}

; write_admission_kernel::write_admission_idle_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel26write_admission_idle_as_is(i1 zeroext %mem_stall, i1 zeroext %pressure_l0, i1 zeroext %stall_l0) unnamed_addr #0 {
start:
  ret i1 true
}

; write_admission_kernel::torn_head_is_empty_log_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel28torn_head_is_empty_log_as_is(i64 %_len, i64 %_tiny_max) unnamed_addr #0 {
start:
  ret i1 true
}

; write_admission_kernel::pit_resync_needs_rewrite_as_is
; Function Attrs: uwtable
define zeroext i1 @_RNvCs95p5YarzxTh_22write_admission_kernel30pit_resync_needs_rewrite_as_is(i1 zeroext %_is_resync) unnamed_addr #0 {
start:
  ret i1 false
}

attributes #0 = { uwtable "frame-pointer"="non-leaf" "probe-stack"="inline-asm" "target-cpu"="apple-m1" }

!llvm.module.flags = !{!0}
!llvm.ident = !{!1}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{!"rustc version 1.97.1 (8bab26f4f 2026-07-14)"}
