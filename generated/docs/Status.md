# Status

## Properties
Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**name** | **String** | Name of the repository, also work as UID | 
**status** | **String** | The current status of the repository. Syncing means it's currently syncing. Waiting means it's waiting for the sync. | 
**next_sync** | **i64** | UTC timestamp for the next sync | 
**last_sync** | **i64** | UTC timestamp of last sync, 0 if never performed | 
**last_result** | **String** | Result of last sync, either \"ok\" or \"failure: reason\". When a sync is never performed \"ok\" is returned. | 
**size** | **i64** | Current size of the repository, in bytes. | 
**packages** | **isize** | Number of packages in the last synchronization. | 

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


