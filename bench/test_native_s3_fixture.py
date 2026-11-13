"""SDK-level fixture contracts; native integration is opt-in, never downloaded."""
import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from botocore.response import StreamingBody

from fixture_probe import assert_bounded, expect_error, read_exact
from sealed_s3_fixture import native_service


class ContractSensitivity(unittest.TestCase):
    def test_full_object_read_amplification_is_rejected(self):
        with self.assertRaisesRegex(AssertionError, 'reads whole object'):
            assert_bounded({'measurements': {'small_ranges': dict(result=8 * 1024**2,
                delta={'rchar': 8 * 1024**3}, rss_increase=0)}})

    def test_object_sized_transient_body_is_rejected(self):
        with self.assertRaisesRegex(AssertionError, 'object-sized body'):
            assert_bounded({'measurements': {'small_ranges': dict(result=8 * 1024**2,
                delta={'rchar': 8 * 1024**2}, rss_increase=128 * 1024**2)}})

    def test_bounded_streaming_cost_is_accepted(self):
        assert_bounded({'measurements': {'small_ranges': dict(result=8 * 1024**2,
            delta={'rchar': 16 * 1024**2}, rss_increase=8 * 1024**2)}})

    def test_reused_rss_cannot_hide_object_sized_go_allocations(self):
        with self.assertRaisesRegex(AssertionError, 'despite reused RSS'):
            assert_bounded({'measurements': {'small_ranges': dict(result=8 * 1024**2,
                delta={'rchar': 16 * 1024**2}, rss_increase=0,
                heap_before={'go_memstats_alloc_bytes_total': 1000},
                heap_after={'go_memstats_alloc_bytes_total': 1000 + 128 * 1024**2})}})

    def test_ignored_if_match_is_rejected(self):
        body = io.BytesIO(b'wrong generation')
        with self.assertRaisesRegex(AssertionError, 'requiring HTTP 412'):
            expect_error(lambda: {'Body': body}, 412)
        self.assertTrue(body.closed)

    def test_wrong_range_bytes_are_rejected_and_closed(self):
        raw = io.BytesIO(b'wrong')
        body = StreamingBody(raw, 5)
        s3 = Mock()
        s3.get_object.return_value = dict(Body=body, ContentLength=5, ContentRange='bytes 0-4/10',
                                        ResponseMetadata={'HTTPStatusCode': 206})
        with self.assertRaisesRegex(AssertionError, 'Range bytes mismatch'):
            read_exact(s3, 'bucket', 'key', '"etag"', 0, 5, 10)
        self.assertTrue(raw.closed)

    def test_ignored_range_status_is_rejected(self):
        body = io.BytesIO(b'whole object')
        s3 = Mock()
        s3.get_object.return_value = dict(Body=body, ContentLength=12, ResponseMetadata={'HTTPStatusCode': 200})
        with self.assertRaises(AssertionError):
            read_exact(s3, 'bucket', 'key', '"etag"', 0, 5, 10)
        self.assertTrue(body.closed)


@unittest.skipUnless(os.environ.get('GEESEFS_WEED_BINARY'), 'explicit locally verified weed binary required')
class NativeContract(unittest.TestCase):
    def test_rw_metadata_listing_conditional_copy_and_multipart(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get('GEESEFS_FIXTURE_TMPDIR')) as temporary:
            with native_service(Path(os.environ['GEESEFS_WEED_BINARY']), Path(temporary)) as (s3, bucket, _, _, _):
                metadata = {'mode': '33184', 'mtime': '1700000000', 'uid': '1000', 'gid': '1000'}
                first = s3.put_object(Bucket=bucket, Key='data/file', Body=b'old', Metadata=metadata)
                head = s3.head_object(Bucket=bucket, Key='data/file', IfMatch=first['ETag'])
                self.assertEqual(head['Metadata'], metadata)
                self.assertEqual(head['ContentLength'], 3)
                s3.copy_object(Bucket=bucket, Key='copy', CopySource={'Bucket': bucket, 'Key': 'data/file'})
                self.assertEqual(s3.head_object(Bucket=bucket, Key='copy')['Metadata'], metadata)
                s3.put_object(Bucket=bucket, Key='data/file', Body=b'new generation')
                expect_error(lambda: s3.get_object(Bucket=bucket, Key='data/file', IfMatch=first['ETag'], Range='bytes=0-1'), 412)
                expect_error(lambda: s3.head_object(Bucket=bucket, Key='data/file', IfMatch=first['ETag']), 412)
                for i in range(5):
                    s3.put_object(Bucket=bucket, Key=f'listing/{i}', Body=b'x')
                pages = list(s3.get_paginator('list_objects_v2').paginate(Bucket=bucket, Prefix='listing/',
                             PaginationConfig={'PageSize': 2}))
                self.assertGreater(len(pages), 1)
                self.assertEqual([item['Key'] for page in pages for item in page.get('Contents', [])],
                                 [f'listing/{i}' for i in range(5)])
                upload = s3.create_multipart_upload(Bucket=bucket, Key='multipart', Metadata=metadata)
                parts = []
                for number, data in [(1, b'a' * (5 * 1024**2)), (2, b'final')]:
                    part = s3.upload_part(Bucket=bucket, Key='multipart', UploadId=upload['UploadId'],
                                          PartNumber=number, Body=data)
                    parts.append(dict(PartNumber=number, ETag=part['ETag']))
                s3.complete_multipart_upload(Bucket=bucket, Key='multipart', UploadId=upload['UploadId'],
                                             MultipartUpload={'Parts': parts})
                response = s3.get_object(Bucket=bucket, Key='multipart', Range='bytes=5242878-5242884')
                with contextlib.closing(response['Body']) as body:
                    self.assertEqual(body.read(), b'aafinal')
                self.assertEqual(s3.head_object(Bucket=bucket, Key='multipart')['Metadata'], metadata)
                aborted = s3.create_multipart_upload(Bucket=bucket, Key='aborted')
                s3.abort_multipart_upload(Bucket=bucket, Key='aborted', UploadId=aborted['UploadId'])
                self.assertEqual(s3.list_multipart_uploads(Bucket=bucket).get('Uploads', []), [])
                s3.delete_object(Bucket=bucket, Key='copy')
                expect_error(lambda: s3.head_object(Bucket=bucket, Key='copy'), 404)

    def test_sealed_restart_preserves_bytes_metadata_and_removes_writer(self):
        def prepare(s3, bucket):
            s3.put_object(Bucket=bucket, Key='payload', Body=b'fixed bytes', Metadata={'mode': '33184'})
        with tempfile.TemporaryDirectory(dir=os.environ.get('GEESEFS_FIXTURE_TMPDIR')) as temporary:
            with native_service(Path(os.environ['GEESEFS_WEED_BINARY']), Path(temporary), prepare, sealed=True) as (s3, bucket, _, _, _):
                response = s3.get_object(Bucket=bucket, Key='payload')
                with contextlib.closing(response['Body']) as body:
                    self.assertEqual(body.read(), b'fixed bytes')
                self.assertEqual(response['Metadata'], {'mode': '33184'})
                expect_error(lambda: s3.put_object(Bucket=bucket, Key='payload', Body=b'changed'), 403)
                expect_error(lambda: s3.delete_object(Bucket=bucket, Key='payload'), 403)


if __name__ == '__main__':
    unittest.main()
