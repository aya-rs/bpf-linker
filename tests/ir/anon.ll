;; This IR was built from the following C code:
;;
;; typedef struct {
;;   int pid;
;; } task;
;;
;; int task_pid(task *t) {
;;   return __builtin_preserve_access_index(t->pid);
;; }
;;
;; The purpose of this code to test whether bpf-linker can handle anonymouts
;; structs which are exposed by named typedefs. We can't do it with Rust.

; ModuleID = 'anon.c'
source_filename = "anon.c"
target datalayout = "e-m:e-p:64:64-i64:64-i128:128-n32:64-S128"
target triple = "bpf"

@"llvm.task:0:0$0:0" = external global i64, !llvm.preserve.access.index !0 #0

; Function Attrs: noinline nounwind optnone
define dso_local i32 @task_pid(ptr noundef %0) #1 !dbg !12 {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  call void @llvm.dbg.declare(metadata ptr %2, metadata !17, metadata !DIExpression()), !dbg !18
  %3 = load ptr, ptr %2, align 8, !dbg !19
  %4 = load i64, ptr @"llvm.task:0:0$0:0", align 8
  %5 = bitcast ptr %3 to ptr
  %6 = getelementptr i8, ptr %5, i64 %4
  %7 = bitcast ptr %6 to ptr
  %8 = call ptr @llvm.bpf.passthrough.p0.p0(i32 0, ptr %7)
  %9 = load i32, ptr %8, align 4, !dbg !20
  ret i32 %9, !dbg !21
}

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare void @llvm.dbg.declare(metadata, metadata, metadata) #2

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(none)
declare ptr @llvm.preserve.struct.access.index.p0.p0(ptr, i32 immarg, i32 immarg) #3

; Function Attrs: nounwind memory(none)
declare ptr @llvm.bpf.passthrough.p0.p0(i32, ptr) #4

attributes #0 = { "btf_ama" }
attributes #1 = { noinline nounwind optnone "frame-pointer"="all" "no-trapping-math"="true" "stack-protector-buffer-size"="8" }
attributes #2 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #3 = { nocallback nofree nosync nounwind willreturn memory(none) }
attributes #4 = { nounwind memory(none) }

!llvm.dbg.cu = !{!6}
!llvm.module.flags = !{!7, !8, !9, !10}
!llvm.ident = !{!11}

!0 = !DIDerivedType(tag: DW_TAG_typedef, name: "task", file: !1, line: 3, baseType: !2)
!1 = !DIFile(filename: "anon.c", directory: "/home/vadorovsky/repos/bpf-linker/tests/ir", checksumkind: CSK_MD5, checksum: "b109ca8da2761a9409803ba2e58fb4a6")
!2 = distinct !DICompositeType(tag: DW_TAG_structure_type, file: !1, line: 1, size: 32, elements: !3)
!3 = !{!4}
!4 = !DIDerivedType(tag: DW_TAG_member, name: "pid", scope: !2, file: !1, line: 2, baseType: !5, size: 32)
!5 = !DIBasicType(name: "int", size: 32, encoding: DW_ATE_signed)
!6 = distinct !DICompileUnit(language: DW_LANG_C11, file: !1, producer: "clang version 17.0.1", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug, splitDebugInlining: false, nameTableKind: None)
!7 = !{i32 7, !"Dwarf Version", i32 5}
!8 = !{i32 2, !"Debug Info Version", i32 3}
!9 = !{i32 1, !"wchar_size", i32 4}
!10 = !{i32 7, !"frame-pointer", i32 2}
!11 = !{!"clang version 17.0.1"}
!12 = distinct !DISubprogram(name: "task_pid", scope: !1, file: !1, line: 5, type: !13, scopeLine: 5, flags: DIFlagPrototyped, spFlags: DISPFlagDefinition, unit: !6, retainedNodes: !16)
!13 = !DISubroutineType(types: !14)
!14 = !{!5, !15}
!15 = !DIDerivedType(tag: DW_TAG_pointer_type, baseType: !0, size: 64)
!16 = !{}
!17 = !DILocalVariable(name: "t", arg: 1, scope: !12, file: !1, line: 5, type: !15)
!18 = !DILocation(line: 5, column: 20, scope: !12)
!19 = !DILocation(line: 6, column: 42, scope: !12)
!20 = !DILocation(line: 6, column: 45, scope: !12)
!21 = !DILocation(line: 6, column: 3, scope: !12)
