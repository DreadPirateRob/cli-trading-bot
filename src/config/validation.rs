use validator::{Validate, ValidationErrors, ValidationErrorsKind};
use crate::error::ConfigError;
use super::types::AppConfig;

/// Two-layer validation: struct constraints + business rules
pub fn validate_config(config: &AppConfig) -> Result<(), ConfigError> {
    // Layer 1: Struct-level validation (ranges, lengths, etc.)
    config.validate().map_err(|e| {
        ConfigError::Validation {
            field: "config".to_string(),
            message: format_validation_errors(e),
        }
    })?;

    // Layer 2: Cross-field business rules
    config.strategy.validate_business_rules().map_err(|msg| {
        ConfigError::Validation {
            field: "strategy".to_string(),
            message: msg,
        }
    })?;

    Ok(())
}

/// Format validation errors with field paths and helpful messages
fn format_validation_errors(errors: ValidationErrors) -> String {
    let mut messages = Vec::new();

    for (field, field_errors) in errors.field_errors() {
        for error in field_errors {
            let msg = error.message
                .as_ref()
                .map(|m| m.to_string())
                .unwrap_or_else(|| format!("invalid value (code: {})", error.code));
            messages.push(format!("{}: {}", field, msg));
        }
    }

    // Handle nested errors
    for (field, nested) in errors.errors() {
        if let ValidationErrorsKind::Struct(box_errors) = nested {
            let nested_msg = format_validation_errors(*box_errors.clone());
            if !nested_msg.is_empty() {
                messages.push(format!("{}.{}", field, nested_msg));
            }
        }
    }

    messages.join("; ")
}
