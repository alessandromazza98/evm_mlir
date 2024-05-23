use std::{ffi::CStr, mem::MaybeUninit, ptr::{addr_of_mut, null_mut}};

use llvm_sys::{core::LLVMDisposeMessage, target_machine::{LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetMachine, LLVMGetDefaultTargetTriple, LLVMGetHostCPUFeatures, LLVMGetHostCPUName, LLVMGetTargetFromTriple, LLVMRelocMode, LLVMTargetRef}};
use melior::{
    dialect::DialectRegistry,
    ir::{
        attribute::StringAttribute, operation::OperationBuilder, Block, Identifier, Location,
        Module as MeliorModule, Region,
    },
    utility::{register_all_dialects, register_all_llvm_translations, register_all_passes},
    Context as MeliorContext,
};

use crate::{codegen::run_pass_manager, opcodes::Opcode};

use super::module::MLIRModule;

#[derive(Debug, Eq, PartialEq)]
pub struct Context {
    melior_context: MeliorContext,
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    pub fn new() -> Self {
        let melior_context = initialize_mlir();
        Self { melior_context }
    }

    pub fn compile(
        &self,
        program: Vec<Opcode>,
    ) -> Result<MLIRModule, String> {
        let location = Location::unknown(&self.melior_context);
        let target_triple = get_target_triple();

        let module_region = Region::new();
        module_region.append_block(Block::new(&[]));

        let data_layout_ret = &get_data_layout_rep()?;

        // build main module
        let op = OperationBuilder::new("builtin.module", location)
            .add_attributes(&[
                (
                    Identifier::new(&self.melior_context, "llvm.target_triple"),
                    StringAttribute::new(&self.melior_context, &target_triple).into(),
                ),
                (
                    Identifier::new(&self.melior_context, "llvm.data_layout"),
                    StringAttribute::new(&self.melior_context, data_layout_ret).into(),
                ),
            ])
            .add_regions([module_region])
            .build().map_err(|_| "failed to build module operation")?;
        assert!(op.verify(), "module operation is not valid");

        let mut melior_module = MeliorModule::from_operation(op).expect("module failed to create");

        // TODO: here we should wire the call to the specific code generation for each opcode

        // let codegen_ctx = CodegenCtx {
        //     mlir_context: &self.melior_context,
        //     session,
        //     mlir_module: &melior_module,
        //     program,
        // };

        // super::codegen::compile_program(codegen_ctx)?;

        assert!(melior_module.as_operation().verify());

        // TODO: Add proper error handling.
        run_pass_manager(&self.melior_context, &mut melior_module).unwrap();

        // The func to llvm pass has a bug where it sets the data layout string to ""
        // This works around it by setting it again.
        {
            let mut op = melior_module.as_operation_mut();
            op.set_attribute(
                "llvm.data_layout",
                StringAttribute::new(&self.melior_context, data_layout_ret).into(),
            );
        }

        // if session.output_mlir {
        //     std::fs::write(
        //         session.output_file.with_extension("after-pass.mlir"),
        //         melior_module.as_operation().to_string(),
        //     )?;
        // }

        Ok(MLIRModule::new(melior_module))
    }
}


/// Initialize an MLIR context.
pub fn initialize_mlir() -> MeliorContext {
    let context = MeliorContext::new();
    context.append_dialect_registry(&{
        let registry = DialectRegistry::new();
        register_all_dialects(&registry);
        registry
    });
    context.load_all_available_dialects();
    register_all_passes();
    register_all_llvm_translations(&context);
    context
}

pub fn get_target_triple() -> String {
    let target_triple = unsafe {
        let value = LLVMGetDefaultTargetTriple();
        CStr::from_ptr(value).to_string_lossy().into_owned()
    };
    target_triple
}

pub fn get_data_layout_rep() -> Result<String, String> {
    unsafe {
        let mut null = null_mut();
        let error_buffer = addr_of_mut!(null);

        let target_triple = LLVMGetDefaultTargetTriple();

        let target_cpu = LLVMGetHostCPUName();

        let target_cpu_features = LLVMGetHostCPUFeatures();

        let mut target: MaybeUninit<LLVMTargetRef> = MaybeUninit::uninit();

        if LLVMGetTargetFromTriple(target_triple, target.as_mut_ptr(), error_buffer) != 0 {
            let error = CStr::from_ptr(*error_buffer);
            let err = error.to_string_lossy().to_string();
            LLVMDisposeMessage(*error_buffer);
            Err(err)?;
        }
        if !(*error_buffer).is_null() {
            LLVMDisposeMessage(*error_buffer);
        }

        let target = target.assume_init();

        let machine = LLVMCreateTargetMachine(
            target,
            target_triple.cast(),
            target_cpu.cast(),
            target_cpu_features.cast(),
            // match session.optlevel {
            //     OptLevel::None => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            //     OptLevel::Less => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            //     OptLevel::Default => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
            //     OptLevel::Aggressive => LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
            // },

            LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            // if session.library {
            //     LLVMRelocMode::LLVMRelocDynamicNoPic
            // } else {
            //     LLVMRelocMode::LLVMRelocDefault
            // },

            LLVMRelocMode::LLVMRelocDefault,
            LLVMCodeModel::LLVMCodeModelDefault,
        );

        let data_layout = llvm_sys::target_machine::LLVMCreateTargetDataLayout(machine);
        let data_layout_str =
            CStr::from_ptr(llvm_sys::target::LLVMCopyStringRepOfTargetData(data_layout));
        Ok(data_layout_str.to_string_lossy().into_owned())
    }
}
