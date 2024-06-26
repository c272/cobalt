use cranelift::codegen::ir::{
    condcodes::{FloatCC, IntCC},
    types, InstBuilder, Value,
};
use miette::Result;

use crate::compiler::{
    codegen::intrinsics::CobaltIntrinsic,
    parser::{self, Cond, IfData},
};

use super::FuncTranslator;

impl<'a, 'src> FuncTranslator<'a, 'src> {
    /// Translates a single "IF" statement to Cranelift IR.
    pub(super) fn translate_if(&mut self, if_data: &IfData<'src>) -> Result<()> {
        // If both the "IF" and "ELSE" blocks contain no statements, we can skip translating this entirely.
        if if_data.if_stats.is_none() && if_data.else_stats.is_none() {
            return Ok(());
        }
        assert!(if_data.if_stats.as_ref().is_some_and(|s| !s.is_empty()));

        // Create blocks for the IF, ELSE and post-statement.
        let if_block = self.builder.create_block();
        let else_block = if_data
            .else_stats
            .is_some()
            .then(|| self.builder.create_block());
        let trailing_block = self.builder.create_block();

        // First, evaluate the contained condition.
        let cond_result = self.translate_cond_eval(&if_data.condition)?;

        // Run the relevant branching instruction.
        let brif_else_block = else_block.as_ref().map_or(trailing_block, |b| *b);
        self.builder
            .ins()
            .brif(cond_result, if_block, &[], brif_else_block, &[]);

        // Seal blocks that have had all their branch instructions defined.
        self.builder.seal_block(if_block);
        if let Some(else_block) = else_block {
            self.builder.seal_block(else_block);
        }

        // Switch to the if block, translate contents.
        // Once the if block is done, we jump to the trailing block.
        self.switch_to_block(if_block);
        let mut block_self_terminates = false;
        for stat in if_data.if_stats.as_ref().unwrap() {
            // If this block has already terminated, we don't want to generate anything else.
            if block_self_terminates {
                miette::bail!("Unreachable statements detected in block: No statements should be placed after unconditional jumps.");
            }
            block_self_terminates |= self.translate_stat(stat)?;
        }
        if !block_self_terminates {
            self.builder.ins().jump(trailing_block, &[]);
        }

        // We can't seal the trailing block yet if we still have an "else" to translate.
        if else_block.is_none() {
            self.builder.seal_block(trailing_block);
        }

        // If there's an else block, translate contents.
        if let Some(else_stats) = if_data.else_stats.as_ref() {
            self.switch_to_block(else_block.unwrap());
            let mut block_self_terminates = false;
            for stat in else_stats {
                // If this block has already terminated, we don't want to generate anything else.
                if block_self_terminates {
                    miette::bail!("Unreachable statements detected in block: No statements should be placed after unconditional jumps.");
                }
                block_self_terminates |= self.translate_stat(stat)?;
            }
            if !block_self_terminates {
                self.builder.ins().jump(trailing_block, &[]);
            }
            self.builder.seal_block(trailing_block);
        }

        // Switch to the trailing block, set as current block.
        self.switch_to_block(trailing_block);
        Ok(())
    }

    /// Translates an evaluation of the given conditional, returning the outcome of the condition.
    /// On the condition being true, the return value is an i64 with a value of 1.
    /// On the condition being false, the return value is an i64 with a value of 0.
    pub(super) fn translate_cond_eval(&mut self, cond: &Cond<'src>) -> Result<Value> {
        // Check that this condition is actually sane.
        self.verify_cond(cond)?;

        match cond {
            // Comparisons.
            Cond::Eq(l, r) => self.translate_cond_comp(l, r, IntCC::Equal, FloatCC::Equal),
            Cond::Ge(l, r) => self.translate_cond_comp(
                l,
                r,
                IntCC::SignedGreaterThanOrEqual,
                FloatCC::GreaterThanOrEqual,
            ),
            Cond::Le(l, r) => self.translate_cond_comp(
                l,
                r,
                IntCC::SignedLessThanOrEqual,
                FloatCC::LessThanOrEqual,
            ),
            Cond::Gt(l, r) => {
                self.translate_cond_comp(l, r, IntCC::SignedGreaterThan, FloatCC::GreaterThan)
            }
            Cond::Lt(l, r) => {
                self.translate_cond_comp(l, r, IntCC::SignedLessThan, FloatCC::LessThan)
            }

            // Recursive conditions.
            Cond::Not(inner) => self.translate_cond_not(inner),
            Cond::And(l, r) => self.translate_cond_and(l, r),
            Cond::Or(l, r) => self.translate_cond_or(l, r),
        }
    }

    /// Translates a single comparison condition into a given value.
    fn translate_cond_comp(
        &mut self,
        l: &parser::Value<'src>,
        r: &parser::Value<'src>,
        int_cc: IntCC,
        float_cc: FloatCC,
    ) -> Result<Value> {
        let (mut l_val, mut r_val) = (self.load_value(l)?, self.load_value(r)?);
        let use_float_cmp = l.is_float(self.data)? || r.is_float(self.data)?;

        // If the comparison requires a floating point comparison, convert both sides to float.
        if use_float_cmp {
            if !l.is_float(self.data)? {
                l_val = self.builder.ins().fcvt_from_sint(types::F64, l_val);
            }
            if !r.is_float(self.data)? {
                r_val = self.builder.ins().fcvt_from_sint(types::F64, r_val);
            }
        }

        // Perform the comparison based on type.
        let result = if l.is_str(self.data)? || r.is_str(self.data)? {
            // String comparison, this must be an equality. We must use our `strcmp` intrinsic.
            assert!(int_cc == IntCC::Equal && float_cc == FloatCC::Equal);
            let strcmp =
                self.intrinsics
                    .get_ref(self.module, self.builder.func, CobaltIntrinsic::StrCmp)?;
            let inst = self.builder.ins().call(strcmp, &[l_val, r_val]);
            *self
                .builder
                .inst_results(inst)
                .first()
                .expect("Strcmp intrinsic does not return a result.")
        } else if use_float_cmp {
            self.builder.ins().fcmp(float_cc, l_val, r_val)
        } else {
            self.builder.ins().icmp(int_cc, l_val, r_val)
        };

        Ok(result)
    }

    /// Translates a single inverted "NOT" condition into Cranelift IR, returning the generated value.
    fn translate_cond_not(&mut self, inner: &Cond<'src>) -> Result<Value> {
        let inner_val = self.translate_cond_eval(inner)?;
        Ok(self.builder.ins().bxor_imm(inner_val, 0x1))
    }

    /// Translates a single combined "AND" condition into Cranelift IR, returning the generated value.
    fn translate_cond_and(&mut self, l: &Cond<'src>, r: &Cond<'src>) -> Result<Value> {
        let (l_val, r_val) = (self.translate_cond_eval(l)?, self.translate_cond_eval(r)?);
        Ok(self.builder.ins().band(l_val, r_val))
    }

    /// Translates a single "OR" condition into Cranelift IR, returning the generated value.
    fn translate_cond_or(&mut self, l: &Cond<'src>, r: &Cond<'src>) -> Result<Value> {
        let (l_val, r_val) = (self.translate_cond_eval(l)?, self.translate_cond_eval(r)?);
        Ok(self.builder.ins().bor(l_val, r_val))
    }

    /// Verifies that the condition provided is sane, and can be computed.
    fn verify_cond(&self, cond: &Cond<'src>) -> Result<()> {
        match cond {
            // Equality comparisons.
            Cond::Eq(left, right) => self.verify_binary_eq_cmp(left, right),

            // Ordinal comparisons.
            Cond::Ge(left, right) => self.verify_binary_ord_cmp(left, right),
            Cond::Le(left, right) => self.verify_binary_ord_cmp(left, right),
            Cond::Gt(left, right) => self.verify_binary_ord_cmp(left, right),
            Cond::Lt(left, right) => self.verify_binary_ord_cmp(left, right),

            // Recursive conditionals.
            Cond::Not(inner) => self.verify_cond(inner),
            Cond::And(l, r) => {
                self.verify_cond(l)?;
                self.verify_cond(r)?;
                Ok(())
            }
            Cond::Or(l, r) => {
                self.verify_cond(l)?;
                self.verify_cond(r)?;
                Ok(())
            }
        }
    }

    /// Verifies that the two values can be checked for equality.
    fn verify_binary_eq_cmp(
        &self,
        left: &parser::Value<'src>,
        right: &parser::Value<'src>,
    ) -> Result<()> {
        if (left.is_str(self.data)? && !right.is_str(self.data)?)
            || (!left.is_str(self.data)? && right.is_str(self.data)?)
        {
            miette::bail!("Cannot compare a string variable to a non-string variable.");
        }
        Ok(())
    }

    /// Verifies that the two values can be compared ordinally.
    fn verify_binary_ord_cmp(
        &self,
        left: &parser::Value<'src>,
        right: &parser::Value<'src>,
    ) -> Result<()> {
        // String types cannot be ordinally compared.
        if left.is_str(self.data)? || right.is_str(self.data)? {
            miette::bail!("Cannot ordinally compare string variables.");
        }
        Ok(())
    }
}
