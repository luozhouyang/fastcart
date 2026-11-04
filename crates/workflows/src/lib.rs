//! Cross-module commerce workflows.
//!
//! Workflows orchestrate isolated module services. They own transaction and
//! idempotency boundaries and never contain provider-specific HTTP code.

pub mod after_sales;
pub mod cart;
pub mod checkout;
pub mod engine;
pub mod maintenance;

pub use after_sales::{
    AfterSalesWorkflow, ClaimRefundResult, ReceiveReturnResult, ReturnRefundResult,
};
pub use cart::{CartContext, CartWorkflow};
pub use checkout::{CheckoutWorkflow, CompleteCheckoutResult, CreateCheckoutRequest};
pub use engine::{
    EngineConfig, StepContext, StepError, WorkflowDefinition, WorkflowEngine, WorkflowExecution,
    WorkflowExecutionDetail, WorkflowExecutionFilter, WorkflowStep, WorkflowStepRecord, status,
};
pub use maintenance::{MaintenanceReport, run_maintenance, run_maintenance_with_options};
