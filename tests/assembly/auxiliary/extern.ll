; ModuleID = 'extern'
; Model a C shim calling a kfunc. Keep the declaration's debug info explicit:
; older Clang omits it at -O0, and newer Clang bitcode may require newer LLVM.
target triple = "bpfel-unknown-none"

@external_variable = external global i32
@weak_variable = extern_weak global i32

define i32 @call_external(i32 %value) !dbg !6 {
  %result = call i32 @external_function(i32 %value), !dbg !7
  %weak_result = call i32 @weak_function(i32 %value), !dbg !7
  %variable = load volatile i32, ptr @external_variable
  %weak_variable = load volatile i32, ptr @weak_variable
  %results = add i32 %result, %weak_result
  %variables = add i32 %variable, %weak_variable
  %sum = add i32 %results, %variables
  ret i32 %sum, !dbg !7
}

declare !dbg !8 i32 @external_function(i32) section ".ksyms"
declare extern_weak i32 @weak_function(i32)

!llvm.dbg.cu = !{!0}
!llvm.module.flags = !{!2}
!0 = distinct !DICompileUnit(language: DW_LANG_C11, file: !1, producer: "bpf-linker test", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!1 = !DIFile(filename: "extern.c", directory: ".")
!2 = !{i32 2, !"Debug Info Version", i32 3}
!3 = !DISubroutineType(types: !4)
!4 = !{!5, !5}
!5 = !DIBasicType(name: "int", size: 32, encoding: DW_ATE_signed)
!6 = distinct !DISubprogram(name: "call_external", scope: !1, file: !1, line: 3, type: !3, spFlags: DISPFlagDefinition, unit: !0)
!7 = !DILocation(line: 4, column: 12, scope: !6)
!8 = !DISubprogram(name: "external_function", scope: !1, file: !1, line: 1, type: !3, spFlags: 0)
