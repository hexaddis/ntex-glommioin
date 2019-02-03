# Changes

## [0.2.1] - 2019-02-xx

### Added

* Add `InOrder` service. the service yields responses as they become available,
  in the order that their originating requests were submitted to the service.

### Changed

* Convert `Timeout` and `InFlight` services to a transforms


## [0.2.0] - 2019-02-01

* Fix framed transport error handling

* Added Clone impl for Either service

* Added Clone impl for Timeout service factory

* Added Service and NewService for Stream dispatcher

* Switch to actix-service 0.2


## [0.1.0] - 2018-12-09

* Move utils services to separate crate
