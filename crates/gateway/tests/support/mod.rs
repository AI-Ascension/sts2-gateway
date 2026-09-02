// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use sts2_gateway::{
    CallerId, Clock, Gateway, GatewayConfig, HealthFault, InstanceId, ProcessFault, ProcessHandle,
    ProcessPort, ProcessState, Readiness, ReadinessPort, SessionId, StopMode, TransportFault,
    TransportPort, TransportRequest, TransportResponse,
};

pub(crate) type TestGateway = Gateway<SharedClock, SharedProcess, SharedReadiness, SharedTransport>;

#[derive(Clone)]
pub(crate) struct SharedClock(Rc<Cell<u64>>);

impl SharedClock {
    fn new() -> Self {
        Self(Rc::new(Cell::new(0)))
    }

    pub(crate) fn advance(&self, millis: u64) {
        self.0.set(self.0.get().saturating_add(millis));
    }
}

impl Clock for SharedClock {
    fn now(&self) -> sts2_gateway::Tick {
        sts2_gateway::Tick::from_millis(self.0.get())
    }
}

struct FakeProcessState {
    next_handle: u64,
    starts: Vec<(InstanceId, ProcessHandle)>,
    statuses: BTreeMap<ProcessHandle, ProcessState>,
    stops: Vec<(ProcessHandle, StopMode)>,
    start_fault: Option<ProcessFault>,
    inspect_fault: Option<ProcessFault>,
    stop_fault: Option<ProcessFault>,
}

#[derive(Clone)]
pub(crate) struct SharedProcess(Rc<RefCell<FakeProcessState>>);

impl SharedProcess {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(FakeProcessState {
            next_handle: 1,
            starts: Vec::new(),
            statuses: BTreeMap::new(),
            stops: Vec::new(),
            start_fault: None,
            inspect_fault: None,
            stop_fault: None,
        })))
    }

    pub(crate) fn set_start_fault(&self, fault: Option<ProcessFault>) {
        self.0.borrow_mut().start_fault = fault;
    }

    pub(crate) fn set_inspect_fault(&self, fault: Option<ProcessFault>) {
        self.0.borrow_mut().inspect_fault = fault;
    }

    pub(crate) fn set_stop_fault(&self, fault: Option<ProcessFault>) {
        self.0.borrow_mut().stop_fault = fault;
    }

    pub(crate) fn handle_for(&self, instance_id: InstanceId) -> Option<ProcessHandle> {
        self.0
            .borrow()
            .starts
            .iter()
            .find(|(started, _)| *started == instance_id)
            .map(|(_, process)| *process)
    }

    pub(crate) fn set_status(&self, process: ProcessHandle, status: ProcessState) {
        self.0.borrow_mut().statuses.insert(process, status);
    }

    pub(crate) fn stop_modes(&self) -> Vec<StopMode> {
        self.0
            .borrow()
            .stops
            .iter()
            .map(|(_, mode)| *mode)
            .collect()
    }
}

impl ProcessPort for SharedProcess {
    fn start(
        &mut self,
        specification: sts2_gateway::LaunchSpec,
    ) -> Result<ProcessHandle, ProcessFault> {
        let mut state = self.0.borrow_mut();
        if let Some(fault) = state.start_fault {
            return Err(fault);
        }
        let process = ProcessHandle::new(state.next_handle);
        state.next_handle = state.next_handle.saturating_add(1);
        state.starts.push((specification.instance_id(), process));
        state.statuses.insert(process, ProcessState::Running);
        Ok(process)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        let state = self.0.borrow();
        if let Some(fault) = state.inspect_fault {
            return Err(fault);
        }
        state
            .statuses
            .get(&process)
            .copied()
            .ok_or(ProcessFault::Unavailable)
    }

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault> {
        let mut state = self.0.borrow_mut();
        state.stops.push((process, mode));
        if let Some(fault) = state.stop_fault {
            return Err(fault);
        }
        state
            .statuses
            .insert(process, ProcessState::Exited { code: None });
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct SharedReadiness(Rc<RefCell<Result<Readiness, HealthFault>>>);

impl SharedReadiness {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Ok(Readiness::Ready))))
    }

    pub(crate) fn set(&self, answer: Result<Readiness, HealthFault>) {
        *self.0.borrow_mut() = answer;
    }
}

impl ReadinessPort for SharedReadiness {
    fn probe(
        &mut self,
        _instance: InstanceId,
        _process: ProcessHandle,
    ) -> Result<Readiness, HealthFault> {
        *self.0.borrow()
    }
}

struct FakeTransportState {
    calls: usize,
    last_request: Option<TransportRequest>,
    response: TransportResponse,
    fault: Option<TransportFault>,
}

#[derive(Clone)]
pub(crate) struct SharedTransport(Rc<RefCell<FakeTransportState>>);

impl SharedTransport {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(FakeTransportState {
            calls: 0,
            last_request: None,
            response: TransportResponse::new(200, vec![1]),
            fault: None,
        })))
    }

    pub(crate) fn set_fault(&self, fault: Option<TransportFault>) {
        self.0.borrow_mut().fault = fault;
    }

    pub(crate) fn calls(&self) -> usize {
        self.0.borrow().calls
    }

    #[allow(dead_code)]
    pub(crate) fn last_request(&self) -> Option<TransportRequest> {
        self.0.borrow().last_request.clone()
    }
}

impl TransportPort for SharedTransport {
    fn forward(&mut self, request: TransportRequest) -> Result<TransportResponse, TransportFault> {
        let mut state = self.0.borrow_mut();
        state.calls += 1;
        state.last_request = Some(request);
        if let Some(fault) = state.fault {
            return Err(fault);
        }
        Ok(state.response.clone())
    }
}

pub(crate) fn new_gateway() -> (
    TestGateway,
    SharedClock,
    SharedProcess,
    SharedReadiness,
    SharedTransport,
) {
    let clock = SharedClock::new();
    let process = SharedProcess::new();
    let readiness = SharedReadiness::new();
    let transport = SharedTransport::new();
    let gateway = Gateway::new(
        GatewayConfig::new(4, 10, 8, 8),
        clock.clone(),
        process.clone(),
        readiness.clone(),
        transport.clone(),
    );
    (gateway, clock, process, readiness, transport)
}

pub(crate) fn owner() -> CallerId {
    CallerId::new(7)
}

pub(crate) fn session() -> SessionId {
    SessionId::new(11)
}

pub(crate) fn ready_gateway(gateway: &mut TestGateway) -> Result<sts2_gateway::Allocation, String> {
    let allocation = gateway
        .allocate(owner(), session())
        .map_err(|error| error.to_string())?;
    gateway
        .reconcile(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    Ok(allocation)
}
